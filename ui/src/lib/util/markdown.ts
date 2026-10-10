/**
 * Markdown-lite → HTML. Safe by construction: every piece of source text is HTML-escaped before
 * any markup is added, and the only tags emitted are a fixed allow-list without attributes
 * (except a constant class on code blocks). Supports fenced code (tolerates an unclosed fence
 * while streaming), inline code, **bold**, *italic*, headings, bullet and numbered lists.
 */
export function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function inline(src: string): string {
  // Split out inline code first so its content is never formatted.
  const parts = src.split(/(`[^`\n]+`)/g);
  return parts
    .map((p) => {
      if (p.length > 1 && p.startsWith("`") && p.endsWith("`")) return `<code>${escapeHtml(p.slice(1, -1))}</code>`;
      let e = escapeHtml(p);
      e = e.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
      e = e.replace(/(^|[^*\w])\*([^*\n]+)\*(?!\*)/g, "$1<em>$2</em>");
      return e;
    })
    .join("");
}

export function renderMarkdown(md: string): string {
  const lines = md.replace(/\r\n?/g, "\n").split("\n");
  const out: string[] = [];
  let para: string[] = [];
  let list: { kind: "ul" | "ol"; items: string[] } | null = null;

  const flushPara = () => {
    if (para.length) out.push(`<p>${para.map(inline).join("<br>")}</p>`);
    para = [];
  };
  const flushList = () => {
    if (list) out.push(`<${list.kind}>${list.items.map((i) => `<li>${inline(i)}</li>`).join("")}</${list.kind}>`);
    list = null;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fence = line.match(/^\s*```\s*([\w+-]*)\s*$/);
    if (fence) {
      flushPara();
      flushList();
      const code: string[] = [];
      i++;
      while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) code.push(lines[i++]);
      out.push(`<pre class="md-code"><code>${escapeHtml(code.join("\n"))}</code></pre>`);
      continue;
    }
    const h = line.match(/^(#{1,4})\s+(.*)$/);
    if (h) {
      flushPara();
      flushList();
      const level = Math.min(h[1].length + 2, 6); // render as h3..h6: chat is not a document
      out.push(`<h${level}>${inline(h[2])}</h${level}>`);
      continue;
    }
    const ul = line.match(/^\s*[-*+]\s+(.*)$/);
    const ol = line.match(/^\s*\d+[.)]\s+(.*)$/);
    if (ul || ol) {
      flushPara();
      const kind = ul ? "ul" : "ol";
      if (list && list.kind !== kind) flushList();
      if (!list) list = { kind, items: [] };
      list.items.push((ul ?? ol)![1]);
      continue;
    }
    if (!line.trim()) {
      flushPara();
      flushList();
      continue;
    }
    flushList();
    para.push(line);
  }
  flushPara();
  flushList();
  return out.join("");
}
