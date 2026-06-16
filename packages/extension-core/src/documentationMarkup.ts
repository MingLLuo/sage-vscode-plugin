function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function renderInline(value: string): string {
  return escapeHtml(value)
    .replace(/``([^`]+)``/g, "<code>$1</code>")
    .replace(/`([^`]+)`/g, "<code>$1</code>");
}

export function renderDocumentationHtml(markdown: string): string {
  const blocks = splitMarkdownBlocks(markdown.trim() || "No documentation available.");
  const title = documentTitleFromBlocks(blocks);
  const htmlBlocks = blocks.map((block) => {
    if (block.startsWith("# ")) {
      return `<h1>${renderInline(block.slice(2).trim())}</h1>`;
    }
    if (block.startsWith("## ")) {
      return `<h2>${renderInline(block.slice(3).trim())}</h2>`;
    }
    if (block.startsWith("> ")) {
      const body = block
        .split("\n")
        .map((line) => line.replace(/^>\s?/, ""))
        .map(renderInline)
        .join("<br />");
      return `<blockquote>${body}</blockquote>`;
    }
    if (block.startsWith("```")) {
      const lines = block.split("\n");
      const firstLine = lines[0] ?? "";
      const language = firstLine.replace(/^```/, "").trim();
      const bodyLines = lines.at(-1)?.trim() === "```" ? lines.slice(1, -1) : lines.slice(1);
      const languageClass = language ? ` class="language-${escapeAttribute(language)}"` : "";
      return `<pre><code${languageClass}>${escapeHtml(bodyLines.join("\n"))}</code></pre>`;
    }
    const lines = block.split("\n");
    if (lines.every((line) => line.startsWith("- "))) {
      return `<ul>${lines
        .map((line) => `<li>${renderInline(line.slice(2).trim())}</li>`)
        .join("")}</ul>`;
    }
    return `<p>${lines.map(renderInline).join("<br />")}</p>`;
  });

  return `<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline';" />
    <style>
      :root {
        color-scheme: light dark;
      }

      body {
        font-family: var(--vscode-font-family);
        color: var(--vscode-editor-foreground);
        background: var(--vscode-editor-background);
        margin: 0;
        line-height: 1.6;
      }

      .shell {
        min-height: 100vh;
        display: grid;
        grid-template-rows: auto 1fr;
      }

      .header {
        position: sticky;
        top: 0;
        z-index: 1;
        border-bottom: 1px solid var(--vscode-panel-border);
        background: var(--vscode-editor-background);
        padding: 16px 22px 14px;
      }

      .eyebrow {
        margin: 0 0 4px;
        color: var(--vscode-descriptionForeground);
        font-size: 11px;
        font-weight: 700;
        letter-spacing: 0;
        text-transform: uppercase;
      }

      .title {
        margin: 0;
        font-size: 18px;
        line-height: 1.3;
        overflow-wrap: anywhere;
      }

      .content {
        max-width: 980px;
        width: 100%;
        padding: 22px;
      }

      .card {
        border: 1px solid var(--vscode-panel-border);
        border-radius: 8px;
        background: color-mix(in srgb, var(--vscode-editor-background) 92%, var(--vscode-editor-foreground));
        padding: 18px;
      }

      h1, h2 {
        margin: 0 0 12px 0;
        line-height: 1.3;
        overflow-wrap: anywhere;
      }

      h1 {
        font-size: 24px;
      }

      h2 {
        font-size: 17px;
        margin-top: 18px;
      }

      p, ul, blockquote, pre {
        margin: 0 0 16px 0;
      }

      code {
        font-family: var(--vscode-editor-font-family);
        background: rgba(127, 127, 127, 0.12);
        padding: 2px 5px;
        border-radius: 6px;
        overflow-wrap: anywhere;
      }

      pre {
        overflow: auto;
        background: rgba(127, 127, 127, 0.12);
        border: 1px solid rgba(127, 127, 127, 0.18);
        border-radius: 8px;
        padding: 12px;
      }

      pre code {
        display: block;
        background: transparent;
        padding: 0;
        border-radius: 0;
        white-space: pre;
      }

      blockquote {
        border-left: 3px solid var(--vscode-textLink-foreground);
        padding-left: 12px;
        color: var(--vscode-descriptionForeground);
      }

      ul {
        padding-left: 22px;
      }

      li + li {
        margin-top: 4px;
      }

      @media (max-width: 720px) {
        .header {
          padding: 14px 16px 12px;
        }

        .content {
          padding: 14px;
        }

        .card {
          border-radius: 6px;
          padding: 14px;
        }
      }
    </style>
    <title>${escapeHtml(title)}</title>
  </head>
  <body>
    <main class="shell">
      <header class="header">
        <p class="eyebrow">Sage Documentation</p>
        <p class="title">${escapeHtml(title)}</p>
      </header>
      <section class="content">
        <article class="card">
          ${htmlBlocks.join("\n")}
        </article>
      </section>
    </main>
  </body>
</html>`;
}

function escapeAttribute(value: string): string {
  return escapeHtml(value).replace(/[^A-Za-z0-9_-]/g, "-");
}

function documentTitleFromBlocks(blocks: string[]): string {
  const heading = blocks.find((block) => block.startsWith("# "));
  return heading?.slice(2).trim() || "Sage Documentation";
}

function splitMarkdownBlocks(markdown: string): string[] {
  const blocks: string[] = [];
  const current: string[] = [];
  let inFence = false;

  const flush = () => {
    if (current.length === 0) {
      return;
    }
    const block = current.join("\n").trim();
    if (block) {
      blocks.push(block);
    }
    current.length = 0;
  };

  for (const line of markdown.split(/\r?\n/)) {
    const isFence = line.trimStart().startsWith("```");
    if (isFence) {
      current.push(line);
      inFence = !inFence;
      if (!inFence) {
        flush();
      }
      continue;
    }

    if (inFence) {
      current.push(line);
      continue;
    }

    if (line.trim() === "") {
      flush();
      continue;
    }
    current.push(line);
  }

  flush();
  return blocks;
}
