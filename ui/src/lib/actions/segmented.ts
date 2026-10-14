/**
 * For a row of choice buttons (`.seg` with `aria-pressed`, or a `tablist` with `aria-selected`):
 * - a thumb slides to the selected button instead of the highlight jumping;
 * - one Tab stop for the whole row (roving tabindex), arrows/Home/End move focus between
 *   buttons, Enter/Space choose. Arrows only move focus: choosing on arrow would let a stray
 *   key switch, say, approval mode to Auto.
 *
 * Behaviour adapted from Interior's segmented-control and tabs (github.com/ddoemonn/interior,
 * MIT). The thumb is placed from the selected button's box, so it follows any markup, buttons
 * of different widths and later changes (the selection, a hidden row being shown, resizing).
 */
export function segmented(node: HTMLElement) {
  const thumb = document.createElement("span");
  thumb.className = "seg-thumb";
  thumb.setAttribute("aria-hidden", "true");
  node.classList.add("has-thumb");
  node.prepend(thumb);

  const buttons = () =>
    [...node.querySelectorAll<HTMLButtonElement>(":scope > button, :scope > [role=tab]")].filter((b) => !b.hidden);
  const isOn = (b: HTMLElement) => b.getAttribute("aria-pressed") === "true" || b.getAttribute("aria-selected") === "true";
  const usable = (b: HTMLElement) => !(b as HTMLButtonElement).disabled && b.getAttribute("aria-disabled") !== "true";

  let placed = false;
  function place() {
    const all = buttons();
    const on = all.find(isOn);
    // Roving tabindex: the selected button (or the first usable one) is the row's Tab stop.
    const stop = on && usable(on) ? on : all.find(usable);
    for (const b of all) {
      b.tabIndex = b === stop ? 0 : -1;
      ro.observe(b);
    }
    if (!on || !on.offsetWidth) {
      thumb.style.opacity = "0";
      return;
    }
    thumb.style.width = `${on.offsetWidth}px`;
    thumb.style.height = `${on.offsetHeight}px`;
    thumb.style.transform = `translate(${on.offsetLeft}px, ${on.offsetTop}px)`;
    thumb.style.opacity = "1";
    // The first placement (and the first after being hidden) does not slide in from 0.
    if (!placed) {
      placed = true;
      thumb.classList.remove("moving");
      requestAnimationFrame(() => thumb.classList.add("moving"));
    }
  }

  function keydown(e: KeyboardEvent) {
    const all = buttons().filter(usable);
    const i = all.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    const vertical = getComputedStyle(node).flexDirection.startsWith("column");
    const next = ({
      ArrowRight: vertical ? null : 1,
      ArrowLeft: vertical ? null : -1,
      ArrowDown: vertical ? 1 : null,
      ArrowUp: vertical ? -1 : null,
    } as Record<string, number | null>)[e.key];
    let to: number | null = null;
    if (next != null) to = (i + next + all.length) % all.length;
    else if (e.key === "Home") to = 0;
    else if (e.key === "End") to = all.length - 1;
    if (to == null) return;
    e.preventDefault();
    all[to].focus();
  }

  const mo = new MutationObserver(place);
  mo.observe(node, { subtree: true, childList: true, attributes: true, attributeFilter: ["aria-pressed", "aria-selected", "hidden", "disabled"] });
  const ro = new ResizeObserver(() => {
    if (!node.offsetWidth) placed = false;
    place();
  });
  ro.observe(node);
  node.addEventListener("keydown", keydown);
  place();

  return {
    destroy() {
      mo.disconnect();
      ro.disconnect();
      node.removeEventListener("keydown", keydown);
      thumb.remove();
      node.classList.remove("has-thumb");
    },
  };
}
