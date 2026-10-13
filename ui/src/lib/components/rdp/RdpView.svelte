<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "../icons/Icon.svelte";
  import { api } from "../../api";
  import type { RdpInput } from "../../api/types";
  import { errorMessage } from "../../api/errors";
  import { t } from "../../i18n";
  import { connections } from "../../stores/connections.svelte";
  import { parseMessage, scancode, WheelAccumulator, type Pointer } from "../../util/rdp";

  /** A remote desktop in a pane: the connection is opened at the pane's size and follows it. */
  let { connectionId, focused }: { connectionId: string; focused: boolean } = $props();

  const conn = $derived(connections.byId.get(connectionId));
  let host = $state<HTMLDivElement>();
  let canvas = $state<HTMLCanvasElement>();
  let phase = $state<"connecting" | "live" | "ended">("connecting");
  let problem = $state<string | null>(null);
  let cursor = $state("default");
  let attempt = $state(0);
  /** The pane no longer matches the desktop's resolution (the desktop is scaled meanwhile). */
  let stale = $state(false);
  /** The server's current pointer, redrawn at the new scale when the pane or desktop size changes. */
  let pointer: Pointer | null = null;

  let id: string | null = null;
  let desk = { width: 0, height: 0 };
  let queue: RdpInput[] = [];
  let flushing = false;
  let lastMove: { x: number; y: number } | null = null;
  const wheel = new WheelAccumulator();

  /** Size in device pixels for the pane; Windows scales its UI by `scale`. */
  function paneSize() {
    const r = host!.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    // Even widths: some servers refuse odd ones.
    const width = Math.max(200, Math.min(8192, Math.floor((r.width * dpr) / 2) * 2));
    const height = Math.max(200, Math.min(8192, Math.floor(r.height * dpr)));
    return { width, height, scale: Math.round(Math.min(Math.max(dpr, 1), 3) * 100) };
  }

  function onMessage(buf: ArrayBuffer) {
    let m;
    try {
      m = parseMessage(buf);
    } catch {
      return;
    }
    if (m.kind === "tiles") {
      const c = canvas!;
      if (c.width !== m.tiles.width || c.height !== m.tiles.height) [c.width, c.height] = [m.tiles.width, m.tiles.height];
      const resized = desk.width !== m.tiles.width || desk.height !== m.tiles.height;
      desk = { width: m.tiles.width, height: m.tiles.height };
      stale = differs(paneSize());
      if (resized) cursor = cssCursor(pointer);
      const ctx = c.getContext("2d")!;
      for (const r of m.tiles.rects) ctx.putImageData(new ImageData(r.data, r.w, r.h), r.x, r.y);
      phase = "live";
      problem = null;
    } else if (m.kind === "pointer") {
      pointer = m.pointer;
      cursor = cssCursor(pointer);
    } else if (m.kind === "error") {
      phase = "ended";
      problem = m.message || t("rdp.failed");
      id = null;
    } else if (m.kind === "terminated") {
      phase = "ended";
      problem = m.error ? m.reason : null;
      id = null;
    }
  }

  /** CSS pixels per desktop pixel: the desktop is scaled (and letterboxed) to the pane. */
  function displayScale() {
    const r = canvas?.getBoundingClientRect();
    const k = r && desk.width && desk.height ? Math.min(r.width / desk.width, r.height / desk.height) : 0;
    return k > 0 ? k : 1 / (window.devicePixelRatio || 1);
  }

  /** The server's pointer at the desktop's on-screen scale, so its tip stays where clicks land. */
  function cssCursor(p: Pointer | null): string {
    if (!p || p.kind === "default") return "default";
    if (p.kind === "hidden") return "none";
    if (!p.width || !p.height || p.width > 256 || p.height > 256) return "default";
    const src = document.createElement("canvas");
    [src.width, src.height] = [p.width, p.height];
    // IronRDP decodes premultiplied alpha; ImageData wants it straight.
    const px = new Uint8ClampedArray(p.rgba);
    for (let i = 0; i < px.length; i += 4) {
      const a = px[i + 3];
      if (a && a < 255) for (let k = 0; k < 3; k++) px[i + k] = Math.min(255, (px[i + k] * 255) / a);
    }
    src.getContext("2d")!.putImageData(new ImageData(px, p.width, p.height), 0, 0);
    // Browsers refuse cursor images over 128 CSS px.
    const k = Math.min(displayScale(), 128 / p.width, 128 / p.height);
    const c = document.createElement("canvas");
    [c.width, c.height] = [Math.max(1, Math.round(p.width * k)), Math.max(1, Math.round(p.height * k))];
    const ctx = c.getContext("2d")!;
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(src, 0, 0, c.width, c.height);
    const hot = (v: number, max: number) => Math.min(max - 1, Math.max(0, Math.round(v * k)));
    return `url(${c.toDataURL()}) ${hot(p.hotX, c.width)} ${hot(p.hotY, c.height)}, default`;
  }

  function differs(s: { width: number; height: number }) {
    return Math.abs(s.width - desk.width) > 8 || Math.abs(s.height - desk.height) > 8;
  }

  /** Reconnect at the pane's size. Windows keeps the session (and its programs) across a
   *  disconnect, so this lands back in the same desktop. IronRDP's live resize (Display Control)
   *  broke the session's bitmap decoding on a Windows Server 2019, so it is not used. */
  async function matchSize() {
    stale = false;
    const old = id;
    if (!old || phase !== "live") return;
    id = null;
    const n = ++attempt;
    await api.rdpClose(old).catch(() => {});
    // Closed meanwhile (the pane's cleanup bumps `attempt`) or reconnected another way.
    if (n !== attempt) return;
    await connect();
  }

  async function connect() {
    phase = "connecting";
    problem = null;
    const n = ++attempt;
    try {
      const newId = await api.rdpOpen(connectionId, paneSize(), (b) => n === attempt && onMessage(b));
      if (n !== attempt) return void api.rdpClose(newId).catch(() => {});
      id = newId;
    } catch (e) {
      if (n !== attempt) return;
      phase = "ended";
      problem = errorMessage(e);
    }
  }

  // ---- input ----
  function send(e: RdpInput) {
    if (!id || phase !== "live") return;
    queue.push(e);
    if (!flushing) {
      flushing = true;
      requestAnimationFrame(flush);
    }
  }
  function flush() {
    flushing = false;
    if (lastMove) {
      queue.unshift({ t: "move", ...lastMove });
      lastMove = null;
    }
    const batch = queue;
    queue = [];
    if (id && batch.length) void api.rdpInput(id, batch).catch(() => {});
  }

  /** Desktop coordinates of a pointer event (the canvas is letterboxed with object-fit). */
  function at(e: MouseEvent) {
    const r = canvas!.getBoundingClientRect();
    const k = Math.min(r.width / desk.width, r.height / desk.height) || 1;
    const [ox, oy] = [(r.width - desk.width * k) / 2, (r.height - desk.height * k) / 2];
    const x = Math.round((e.clientX - r.left - ox) / k);
    const y = Math.round((e.clientY - r.top - oy) / k);
    return { x: Math.max(0, Math.min(desk.width - 1, x)), y: Math.max(0, Math.min(desk.height - 1, y)) };
  }
  function move(e: PointerEvent) {
    if (!id || phase !== "live") return;
    lastMove = at(e);
    if (!flushing) {
      flushing = true;
      requestAnimationFrame(flush);
    }
  }
  function button(e: PointerEvent, down: boolean) {
    if (phase !== "live") return;
    e.preventDefault();
    if (down) {
      canvas!.focus();
      canvas!.setPointerCapture(e.pointerId);
    }
    lastMove = at(e);
    send({ t: down ? "down" : "up", button: e.button });
  }
  function onwheel(e: WheelEvent) {
    if (phase !== "live") return;
    e.preventDefault();
    const v = wheel.push(e.deltaY, e.deltaMode);
    if (v) send({ t: "wheel", vertical: true, units: v });
    if (e.deltaX) {
      const h = wheel.push(e.deltaX, e.deltaMode);
      if (h) send({ t: "wheel", vertical: false, units: -h });
    }
  }
  function key(e: KeyboardEvent, down: boolean) {
    if (phase !== "live") return;
    const code = scancode(e.code);
    if (code == null) return;
    e.preventDefault();
    e.stopPropagation();
    send({ t: "key", code, down });
  }

  // Keyboard focus follows the pane.
  $effect(() => {
    if (focused && phase === "live") canvas?.focus();
  });

  onMount(() => {
    void connect();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ro = new ResizeObserver(() => {
      clearTimeout(timer);
      // The desktop scales to the pane; changing its resolution can cost a reconnect, so that
      // waits for the "Match window size" button.
      timer = setTimeout(() => {
        if (id && phase === "live") {
          stale = differs(paneSize());
          cursor = cssCursor(pointer);
        }
      }, 400);
    });
    ro.observe(host!);
    return () => {
      ro.disconnect();
      clearTimeout(timer);
      attempt++;
      if (id) void api.rdpClose(id).catch(() => {});
      id = null;
    };
  });
</script>

<div class="rdp" bind:this={host}>
  <canvas
    bind:this={canvas}
    class:hidden={phase !== "live"}
    tabindex="0"
    aria-label={t("rdp.desktop", { name: conn?.name ?? "" })}
    style:cursor
    onpointermove={move}
    onpointerdown={(e) => button(e, true)}
    onpointerup={(e) => button(e, false)}
    onwheel={onwheel}
    onkeydown={(e) => key(e, true)}
    onkeyup={(e) => key(e, false)}
    onblur={() => send({ t: "release_all" })}
    oncontextmenu={(e) => e.preventDefault()}
  ></canvas>
  {#if phase === "live" && stale}
    <button class="btn sm fit" onclick={matchSize} title={t("rdp.matchSizeHint")}><Icon name="refresh" size={12} />{t("rdp.matchSize")}</button>
  {/if}
  {#if phase !== "live"}
    <div class="state" role="status">
      <Icon name="monitor" size={28} />
      {#if phase === "connecting"}
        <p>{t("rdp.connecting", { name: conn?.name ?? "" })}</p>
        <p class="hint mono">{conn ? `${conn.user}@${conn.host}${conn.port !== 3389 ? `:${conn.port}` : ""}` : ""}</p>
      {:else}
        <p>{problem ? t("rdp.failedTitle", { name: conn?.name ?? "" }) : t("rdp.ended")}</p>
        {#if problem}<p class="err">{problem}</p>{/if}
        <button class="btn primary sm" onclick={connect}><Icon name="refresh" size={13} />{t("rdp.reconnect")}</button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .rdp {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: #000;
    display: grid;
    place-items: center;
  }
  canvas {
    max-width: 100%;
    max-height: 100%;
    width: 100%;
    height: 100%;
    object-fit: contain;
    outline: none;
    touch-action: none;
  }
  canvas.hidden {
    visibility: hidden;
  }
  .fit {
    position: absolute;
    top: 8px;
    right: 8px;
    opacity: 0.85;
  }
  .fit:hover {
    opacity: 1;
  }
  .state {
    position: absolute;
    inset: 0;
    display: grid;
    place-content: center;
    justify-items: center;
    gap: 8px;
    padding: 24px;
    text-align: center;
    color: var(--muted);
    background: var(--bg);
  }
  .state p {
    margin: 0;
  }
  .err {
    max-width: 460px;
    color: var(--danger);
    font-size: var(--fs-meta);
    overflow-wrap: anywhere;
  }
</style>
