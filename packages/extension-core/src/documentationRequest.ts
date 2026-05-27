export interface DocumentationRequestPayload {
  textDocument: {
    uri: string;
  };
  position: {
    line: number;
    character: number;
  };
  symbol?: string;
}

export interface DocumentationResult {
  symbol: string;
  module?: string;
  kind?: string;
  detail?: string;
  summary: string;
  docstring?: string;
  uri?: string;
  markers: string[];
  sections: Array<{
    title: string;
    body: string;
  }>;
}

export interface DocumentationResponse {
  name: string;
  moduleName?: string;
  kind?: string;
  detail?: string;
  summary?: string;
  docstring?: string;
  uri?: string;
  markers?: string[];
  sections?: Array<{
    title: string;
    body: string;
  }>;
}

export function buildDocumentationRequestPayload(
  uri: string,
  line: number,
  character: number,
  symbol?: string,
): DocumentationRequestPayload {
  return {
    textDocument: { uri },
    position: { line, character },
    ...(symbol ? { symbol } : {}),
  };
}

export function renderDocumentationMarkdown(result: DocumentationResult): string {
  const lines = [`# ${result.symbol}`, ""];

  if (result.detail) {
    lines.push(result.detail);
  }

  if (result.module) {
    lines.push(`Module: \`${result.module}\``);
  }

  if (result.kind) {
    lines.push(`Kind: ${result.kind}`);
  }

  if (result.uri) {
    lines.push(`Source: ${result.uri}`);
  }

  if (result.markers.length > 0) {
    lines.push("");
    lines.push(`> ${result.markers.map((marker) => `\`${marker}\``).join(" ")}`);
  }

  if (lines.length > 2) {
    lines.push("");
  }

  lines.push(result.summary);

  if (result.docstring && result.docstring.trim() && result.docstring.trim() !== result.summary.trim()) {
    const docstring = result.docstring.trim().startsWith(result.summary.trim())
      ? result.docstring.trim().slice(result.summary.trim().length).trim()
      : result.docstring.trim();
    if (docstring) {
      lines.push("");
      lines.push(formatSageDocstringMarkdown(docstring));
    }
  }

  for (const section of result.sections) {
    lines.push("");
    lines.push(`## ${section.title}`);
    lines.push("");
    lines.push(formatSageDocstringMarkdown(section.body));
  }

  return lines.join("\n");
}

export function formatSageDocstringMarkdown(value: string): string {
  const input = dedentSageDocstring(value.trim());
  const lines = input.split(/\r?\n/);
  const output: string[] = [];
  let inLiteralBlock = false;
  let awaitingLiteralBlock = false;

  const closeLiteralBlock = () => {
    if (inLiteralBlock) {
      output.push("```");
      inLiteralBlock = false;
    }
    awaitingLiteralBlock = false;
  };

  const openLiteralBlock = () => {
    if (!inLiteralBlock) {
      if (output.length > 0 && output.at(-1) !== "") {
        output.push("");
      }
      output.push("```sage");
      inLiteralBlock = true;
    }
  };

  for (const rawLine of lines) {
    const line = rawLine.trimEnd();
    const stripped = line.trimStart();
    const indent = line.length - stripped.length;

    if (/::$/.test(stripped) && !stripped.startsWith(".. ")) {
      closeLiteralBlock();
      const heading = stripped.replace(/::\s*$/, ":");
      if (heading) {
        output.push(heading);
      }
      awaitingLiteralBlock = true;
      continue;
    }

    const doctestLine = /^(sage:|\.\.\.:)/.test(stripped);
    const indentedLiteralLine = awaitingLiteralBlock && (line.length === 0 || indent >= 4);
    if (doctestLine || indentedLiteralLine) {
      if (line.length === 0 && !inLiteralBlock) {
        continue;
      }
      openLiteralBlock();
      output.push(indentedLiteralLine && indent >= 4 ? line.slice(4) : stripped);
      continue;
    }

    closeLiteralBlock();
    output.push(stripped);
  }

  closeLiteralBlock();
  return output.join("\n").replace(/\n{3,}/g, "\n\n").trim();
}

function dedentSageDocstring(value: string): string {
  const lines = value.split(/\r?\n/);
  const tailIndents = lines
    .slice(1)
    .filter((line) => line.trim().length > 0)
    .map((line) => line.length - line.trimStart().length);
  const indent = Math.min(...tailIndents, Number.POSITIVE_INFINITY);
  if (!Number.isFinite(indent) || indent <= 0) {
    return value;
  }
  return lines
    .map((line, index) => (index > 0 && line.startsWith(" ".repeat(indent)) ? line.slice(indent) : line))
    .join("\n");
}

export function normalizeDocumentationResponse(
  response: DocumentationResponse | null,
): DocumentationResult | null {
  if (!response) {
    return null;
  }

  const fallbackSummary = response.docstring
    ?.split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line.length > 0);

  return {
    symbol: response.name,
    module: response.moduleName,
    kind: response.kind,
    detail: response.detail,
    summary: response.summary ?? fallbackSummary ?? response.detail ?? "No documentation available.",
    docstring: response.docstring,
    uri: response.uri,
    markers: response.markers ?? [],
    sections: response.sections ?? [],
  };
}
