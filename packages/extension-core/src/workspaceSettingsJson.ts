import {
  applyEdits,
  modify,
  parseTree,
  printParseErrorCode,
  type ParseError,
} from "jsonc-parser";

export function updateWorkspaceSettingJson(
  source: string,
  setting: string,
  value: unknown,
): string {
  const bom = source.startsWith("\uFEFF") ? "\uFEFF" : "";
  const sourceWithoutBom = bom ? source.slice(bom.length) : source;
  const normalizedSource = sourceWithoutBom.trim().length > 0 ? sourceWithoutBom : "{}\n";
  const errors: ParseError[] = [];
  const root = parseTree(normalizedSource, errors, {
    allowTrailingComma: true,
    disallowComments: false,
  });
  if (errors.length > 0) {
    const first = errors[0];
    throw new Error(
      `Cannot update workspace settings because settings.json is invalid near offset ${first.offset + bom.length}: ${printParseErrorCode(first.error)}`,
    );
  }
  if (!root || root.type !== "object") {
    throw new Error("Cannot update workspace settings because settings.json must contain an object.");
  }
  const matchingProperties = (root.children ?? []).filter(
    (property) => property.type === "property" && property.children?.[0]?.value === setting,
  );
  if (matchingProperties.length > 1) {
    throw new Error(
      `Cannot safely update ${setting} because settings.json contains the setting more than once.`,
    );
  }

  const eol = normalizedSource.includes("\r\n") ? "\r\n" : "\n";
  const indentation = detectIndentation(normalizedSource);
  const edits = modify(normalizedSource, [setting], value, {
    formattingOptions: {
      insertSpaces: indentation !== "\t",
      tabSize: indentation === "\t" ? 1 : indentation.length,
      eol,
    },
  });
  const updated = applyEdits(normalizedSource, edits);
  const updatedWithEol = updated.endsWith(eol) ? updated : `${updated}${eol}`;
  return `${bom}${updatedWithEol}`;
}

function detectIndentation(source: string): string {
  for (const line of source.split(/\r?\n/).slice(1)) {
    const match = /^(\s+)["}]/.exec(line);
    if (match?.[1]) {
      return match[1].includes("\t") ? "\t" : match[1];
    }
  }
  return "  ";
}
