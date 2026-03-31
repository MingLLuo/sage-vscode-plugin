function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");
}

function renderInline(value: string): string {
  return escapeHtml(value).replace(/`([^`]+)`/g, "<code>$1</code>");
}

export function renderDocumentationHtml(markdown: string): string {
  const blocks = markdown.split(/\n{2,}/).map((block) => block.trim()).filter(Boolean);
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

      p, ul, blockquote {
        margin: 0 0 16px 0;
      }

      code {
        font-family: var(--vscode-editor-font-family);
        background: rgba(127, 127, 127, 0.12);
        padding: 2px 6px;
        border-radius: 6px;
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
