// Integrity landing page. Renders the real engineering JOURNAL (imported as raw markdown at build time)
// and stamps the build id, so the page itself proves it's the freshly-shipped copy (no stale Safari cache).
// No WASM/GPU here — this is just the front door.

// Vite inlines the repo-root JOURNAL.md as a string (fs.allow includes "..").
import journalRaw from "../../JOURNAL.md?raw";

function esc(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

// Inline formatting: code spans, then bold, then italics. (Escape HTML first so the journal's own
// `<angle>` snippets render literally.)
function inline(s: string): string {
  return esc(s)
    .replace(/`([^`]+)`/g, "<code>$1</code>")
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*])\*([^*]+)\*/g, "$1<em>$2</em>");
}

// A deliberately small markdown renderer covering exactly what the journal uses: h1–h3, ---, bold/code/
// italic, unordered + ordered lists, GFM tables, and paragraphs. Not a general parser — our own content.
function renderMarkdown(md: string): string {
  const lines = md.split("\n");
  const out: string[] = [];
  let i = 0;
  let listType: "ul" | "ol" | null = null;
  const closeList = (): void => {
    if (listType) {
      out.push(`</${listType}>`);
      listType = null;
    }
  };
  const isBlock = (l: string): boolean => /^(#{1,3}\s|[-*]\s|\d+\.\s|\||---\s*$|\s*$)/.test(l);

  while (i < lines.length) {
    const line = lines[i];

    // Table block: consecutive lines starting with "|".
    if (/^\s*\|/.test(line)) {
      closeList();
      const rows: string[] = [];
      while (i < lines.length && /^\s*\|/.test(lines[i])) rows.push(lines[i++]);
      const cells = (r: string): string[] =>
        r.trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
      out.push("<table>");
      rows.forEach((r, idx) => {
        if (idx === 1 && /^[\s|:-]+$/.test(r)) return; // header separator row
        const tag = idx === 0 ? "th" : "td";
        out.push("<tr>" + cells(r).map((c) => `<${tag}>${inline(c)}</${tag}>`).join("") + "</tr>");
      });
      out.push("</table>");
      continue;
    }

    let m: RegExpExecArray | null;
    if ((m = /^###\s+(.*)/.exec(line))) {
      closeList();
      out.push(`<h3>${inline(m[1])}</h3>`);
      i++;
      continue;
    }
    if ((m = /^##\s+(.*)/.exec(line))) {
      closeList();
      out.push(`<h2>${inline(m[1])}</h2>`);
      i++;
      continue;
    }
    if ((m = /^#\s+(.*)/.exec(line))) {
      closeList();
      out.push(`<h1>${inline(m[1])}</h1>`);
      i++;
      continue;
    }
    if (/^---\s*$/.test(line)) {
      closeList();
      out.push("<hr>");
      i++;
      continue;
    }
    const um = /^[-*]\s+(.*)/.exec(line);
    const om = /^\d+\.\s+(.*)/.exec(line);
    if (um || om) {
      const want = um ? "ul" : "ol";
      if (listType !== want) {
        closeList();
        out.push(`<${want}>`);
        listType = want;
      }
      out.push(`<li>${inline((um ?? om)![1])}</li>`);
      i++;
      continue;
    }
    if (/^\s*$/.test(line)) {
      closeList();
      i++;
      continue;
    }
    // Paragraph: gather until a blank line or the next block element.
    closeList();
    const para: string[] = [line];
    i++;
    while (i < lines.length && !isBlock(lines[i])) para.push(lines[i++]);
    out.push(`<p>${inline(para.join(" "))}</p>`);
  }
  closeList();
  return out.join("\n");
}

const body = document.getElementById("journal-body");
if (body) body.innerHTML = renderMarkdown(journalRaw);

const stamp = document.getElementById("build-stamp");
if (stamp) stamp.textContent = `build ${__BUILD_ID__}`;
