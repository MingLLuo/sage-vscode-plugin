function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function renderInline(value: string): string {
  return escapeHtml(value)
    .replace(/``([^`]+)``/g, "<code>$1</code>")
    .replace(/`([^`]+)`/g, "<code>$1</code>");
}

export function renderDocumentationHtml(markdown: string): string {
  const blocks = splitMarkdownBlocks(markdown);
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
      const languageClass = language ? ` class="language-${escapeHtml(language)}"` : "";
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
    <style>
      body {
        font-family: var(--vscode-font-family);
        color: var(--vscode-editor-foreground);
        background: linear-gradient(135deg, rgba(229, 242, 255, 0.12), transparent 45%),
          var(--vscode-editor-background);
        margin: 0;
        padding: 24px;
        line-height: 1.6;
      }

      h1, h2 {
        margin: 0 0 12px 0;
      }

      p, ul, blockquote, pre {
        margin: 0 0 16px 0;
      }

      code {
        font-family: var(--vscode-editor-font-family);
        background: rgba(127, 127, 127, 0.12);
        padding: 2px 6px;
        border-radius: 6px;
      }

      pre {
        overflow: auto;
        background: rgba(127, 127, 127, 0.12);
        border: 1px solid rgba(127, 127, 127, 0.18);
        border-radius: 6px;
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
    </style>
  </head>
  <body>
    ${htmlBlocks.join("\n")}
  </body>
</html>`;
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
