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

  for (const section of result.sections) {
    lines.push("");
    lines.push(`## ${section.title}`);
    lines.push("");
    lines.push(section.body);
  }

  return lines.join("\n");
}

export function normalizeDocumentationResponse(
  response: DocumentationResponse | null,
): DocumentationResult | null {
  if (!response) {
    return null;
  }

  return {
    symbol: response.name,
    module: response.moduleName,
    kind: response.kind,
    detail: response.detail,
    summary: response.summary ?? response.docstring ?? response.detail ?? "No documentation available.",
    uri: response.uri,
    markers: response.markers ?? [],
    sections: response.sections ?? [],
  };
}
