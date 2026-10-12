<script lang="ts" module>
  // Lucide-style 24×24 outline glyphs, drawn at 1.5px stroke.
  const P = {
    server: "M4 4h16v6H4zM4 14h16v6H4zM8 7h.01M8 17h.01",
    monitor: "M3 4h18v12H3zM8 20h8M12 16v4",
    snippet: "M8 6 3 12l5 6M16 6l5 6-5 6",
    settings:
      "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6ZM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z",
    sparkle: "M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8ZM19 15l.7 2 2 .7-2 .7-.7 2-.7-2-2-.7 2-.7Z",
    search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14ZM20 20l-4-4",
    plus: "M12 5v14M5 12h14",
    x: "M18 6 6 18M6 6l12 12",
    chevronRight: "m9 6 6 6-6 6",
    chevronDown: "m6 9 6 6 6-6",
    more: "M6 12h.01M12 12h.01M18 12h.01",
    folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z",
    folderPlus: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2ZM12 11v5M9.5 13.5h5",
    terminal: "M4 5h16v14H4zM7 9l3 3-3 3M12 15h5",
    splitRight: "M4 5h16v14H4zM12 5v14",
    splitDown: "M4 5h16v14H4zM4 12h16",
    lock: "M6 11h12v9H6zM8.5 11V8a3.5 3.5 0 0 1 7 0v3",
    key: "M14.5 9.5a4 4 0 1 1-1.2-2.9M14.5 9.5 21 16l-2 2-1.5-1.5L16 18l-1.5-1.5L16 15l-2.9-2.9",
    copy: "M9 9h11v11H9zM5 15H4V4h11v1",
    download: "M12 4v11M7 10l5 5 5-5M5 20h14",
    upload: "M12 20V9M7 14l5-5 5 5M5 4h14",
    arrowUp: "M12 19V5M6 11l6-6 6 6",
    file: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8ZM14 3v5h5",
    filePlus: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8ZM14 3v5h5M12 11v6M9 14h6",
    check: "m5 12 5 5 9-10",
    alert: "M12 4 2.5 20h19ZM12 10v4M12 17h.01",
    shield: "M12 3 5 6v5c0 4.5 3 8.3 7 10 4-1.7 7-5.5 7-10V6Z",
    send: "M5 12h13M13 6l6 6-6 6",
    stop: "M7 7h10v10H7z",
    edit: "M4 20h4L19 9l-4-4L4 16ZM13.5 6.5l4 4",
    trash: "M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13",
    pin: "M9 4h6l-1 5 3 3v2H7v-2l3-3ZM12 14v6",
    play: "M7 5v14l11-7Z",
    clock: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM12 7v5l3 2",
    bolt: "M13 3 5 14h6l-1 7 8-11h-6Z",
    layout: "M4 5h16v14H4zM10 5v14M10 12h10",
    menu: "M4 7h16M4 12h16M4 17h16",
    jump: "M7 17 17 7M9 7h8v8",
    bucket: "M4 6c0 1.1 3.6 2 8 2s8-.9 8-2-3.6-2-8-2-8 .9-8 2ZM4 6l2 13c.2 1.1 2.9 2 6 2s5.8-.9 6-2l2-13",
    cloud: "M7 18a4 4 0 0 1-.6-8A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9Z",
    palette: "M12 3a9 9 0 1 0 0 18c1 0 1.5-.7 1.5-1.5 0-.4-.2-.8-.4-1.1-.3-.3-.4-.7-.4-1.1 0-.8.7-1.5 1.5-1.5H16a5 5 0 0 0 5-5c0-4.1-4-7.8-9-7.8ZM7.5 11.5h.01M10 7.5h.01M14.5 7.5h.01",
    info: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM12 11v5M12 8h.01",
    import: "M12 3v12M7 10l5 5 5-5M4 15v4h16v-4",
    refresh: "M20 11a8 8 0 0 0-14.7-4.3L4 8M4 4v4h4M4 13a8 8 0 0 0 14.7 4.3L20 16M20 20v-4h-4",
    link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1",
  } as const;
  export type IconName = keyof typeof P;
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"><path d={P[name]} /></svg>
