export interface SageCell {
  startLine: number;
  endLine: number;
  text: string;
}

export interface SageCellMarker {
  line: number;
  kind: "cell" | "region";
  label: string;
}

export function sageCellMarkers(text: string): SageCellMarker[] {
  const markers: SageCellMarker[] = [];
  for (const [index, line] of text.split(/\r?\n/).entries()) {
    const cellMatch = line.match(/^\s*#\s*%%\s*(.*)$/);
    if (cellMatch) {
      markers.push({
        line: index,
        kind: "cell",
        label: cellMatch[1]?.trim() || "Sage cell",
      });
      continue;
    }

    const regionMatch = line.match(/^\s*#\s*region\b\s*(.*)$/i);
    if (regionMatch) {
      markers.push({
        line: index,
        kind: "region",
        label: regionMatch[1]?.trim() || "Sage region",
      });
    }
  }
  return markers;
}

export function currentSageCell(text: string, activeLine: number): SageCell | undefined {
  const lines = text.split(/\r?\n/);
  if (lines.length === 0) {
    return undefined;
  }

  const clampedLine = Math.max(0, Math.min(activeLine, lines.length - 1));
  let startLine = 0;
  for (let line = clampedLine; line >= 0; line -= 1) {
    if (isSageCellBoundary(lines[line])) {
      startLine = line + 1;
      break;
    }
  }

  let endLine = lines.length - 1;
  for (let line = clampedLine + 1; line < lines.length; line += 1) {
    if (isSageCellBoundary(lines[line])) {
      endLine = line - 1;
      break;
    }
  }

  while (startLine <= endLine && lines[startLine].trim() === "") {
    startLine += 1;
  }
  while (endLine >= startLine && lines[endLine].trim() === "") {
    endLine -= 1;
  }
  if (startLine > endLine) {
    return undefined;
  }

  return {
    startLine,
    endLine,
    text: lines.slice(startLine, endLine + 1).join("\n"),
  };
}

function isSageCellBoundary(line: string): boolean {
  return /^\s*#\s*%%/.test(line)
    || /^\s*#\s*region\b/i.test(line)
    || /^\s*#\s*endregion\b/i.test(line);
}
