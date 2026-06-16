"use strict";

const manifest = window.__SAGE_REFERENCE_MANIFEST__;
const data = window.__SAGE_REFERENCE_SYMBOLS__;
window.__SAGE_REFERENCE_SOURCES__ = window.__SAGE_REFERENCE_SOURCES__ || {};

const state = { symbolId: null, sourceId: null, line: null, recent: [] };
const byId = new Map(data.symbols.map((symbol) => [symbol.id, symbol]));
const sources = new Map(data.sources.map((source) => [source.id, source]));
const referencesBySymbol = new Map();
for (const reference of data.references) {
  if (!referencesBySymbol.has(reference.symbolId)) {
    referencesBySymbol.set(reference.symbolId, []);
  }
  referencesBySymbol.get(reference.symbolId).push(reference);
}

const projectName = document.querySelector("#projectName");
const projectMeta = document.querySelector("#projectMeta");
const searchInput = document.querySelector("#searchInput");
const symbolGroups = document.querySelector("#symbolGroups");
const resultCount = document.querySelector("#resultCount");
const themeButton = document.querySelector("#themeButton");

projectName.textContent = manifest.projectName + " Sage Reference";
projectMeta.textContent = [
  manifest.stats.symbols + " symbols",
  manifest.stats.sources + " sources",
  manifest.stats.references + " references",
  "generated " + formatGeneratedAt(manifest.generatedAt),
].join(" | ");

searchInput.addEventListener("input", () => renderSymbolGroups(searchInput.value));
searchInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    const first = document.querySelector(".symbol-item");
    if (first) {
      first.click();
    }
  }
  if (event.key === "Escape") {
    searchInput.value = "";
    renderSymbolGroups("");
  }
});

themeButton.addEventListener("click", () => {
  const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
  document.documentElement.dataset.theme = next;
  localStorage.setItem("sage-reference-theme", next);
});

for (const button of document.querySelectorAll("[data-panel]")) {
  button.addEventListener("click", () => setPanel(button.dataset.panel));
}

document.addEventListener("keydown", (event) => {
  if (event.key === "/" && document.activeElement !== searchInput) {
    event.preventDefault();
    searchInput.focus();
  }
  if (event.key === "[" || event.key === "]") {
    moveReference(event.key === "]" ? 1 : -1);
  }
});

window.addEventListener("hashchange", restoreFromHash);
document.documentElement.dataset.theme = localStorage.getItem("sage-reference-theme")
  || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
setPanel("source");
renderSymbolGroups("");
restoreFromHash();
if (state.symbolId == null && data.symbols[0]) {
  selectSymbol(data.symbols[0].id, false);
}

function renderSymbolGroups(filter) {
  const needle = filter.trim().toLowerCase();
  const ids = needle
    ? data.searchIndex.filter((entry) => fuzzy(entry.text, needle)).map((entry) => entry.id)
    : data.symbols.map((symbol) => symbol.id);
  const groups = new Map();
  for (const id of ids.slice(0, 500)) {
    const symbol = byId.get(id);
    if (!symbol) {
      continue;
    }
    const group = groupFor(symbol);
    if (!groups.has(group)) {
      groups.set(group, []);
    }
    groups.get(group).push(symbol);
  }
  resultCount.textContent = String(ids.length);
  symbolGroups.innerHTML = "";
  if (ids.length === 0) {
    symbolGroups.innerHTML = '<div class="empty">No matching symbols.</div>';
    return;
  }
  for (const [group, symbols] of groups) {
    const title = document.createElement("div");
    title.className = "group-title";
    title.textContent = group;
    symbolGroups.append(title);
    for (const symbol of symbols) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "symbol-item" + (symbol.id === state.symbolId ? " active" : "");
      button.dataset.symbolId = String(symbol.id);
      button.innerHTML = '<span class="symbol-name origin-' + escapeAttr(symbol.origin) + '">' + escapeHtml(symbol.name) + '</span>'
        + '<span class="symbol-meta">' + escapeHtml([symbol.kind, symbol.module].filter(Boolean).join(" | ")) + '</span>';
      button.addEventListener("click", () => selectSymbol(symbol.id, true));
      symbolGroups.append(button);
    }
  }
}

async function selectSymbol(id, updateHash) {
  const symbol = byId.get(id);
  if (!symbol) {
    return;
  }
  state.symbolId = id;
  state.sourceId = symbol.definition?.sourceId ?? symbol.sourceId;
  state.line = symbol.definition?.range?.startLine ?? null;
  state.recent = [id, ...state.recent.filter((entry) => entry !== id)].slice(0, 8);
  if (updateHash) {
    setHash("symbol=" + encodeURIComponent(symbol.name));
  }
  renderDetail(symbol);
  renderReferences(symbol);
  renderRecent();
  setPanel("source");
  await renderSource(state.sourceId, state.line, id);
  renderSymbolGroups(searchInput.value);
}

function renderDetail(symbol) {
  const target = document.querySelector("#detailView");
  const definition = symbol.definition;
  target.innerHTML =
    '<h2>' + escapeHtml(symbol.name) + '</h2>' +
    '<div class="badge-row">' +
    badge(symbol.origin) + badge(symbol.kind) + badge(symbol.confidence, symbol.confidence !== "high") + badge(symbol.ownerType || "") +
    '</div>' +
    '<p>' + escapeHtml(symbol.module || "module unavailable") + '</p>' +
    '<div class="action-row">' +
    actionButton("Copy symbol", symbol.name) +
    actionButton("Copy path", definition?.virtualPath || "") +
    actionButton("Copy definition", definition ? definition.virtualPath + ":" + (definition.range.startLine + 1) : "") +
    '</div>' +
    (symbol.signature ? '<div class="signature">' + escapeHtml(symbol.signature) + '</div>' : '<p>No signature available.</p>') +
    '<div class="full-doc">' + escapeHtml(symbol.doc || symbol.summary || "No documentation was exported for this symbol.") + '</div>' +
    '<p>Resolution: ' + escapeHtml(symbol.reason || "none") + '</p>';
  for (const button of target.querySelectorAll("button[data-action]")) {
    button.addEventListener("click", async () => {
      await copyText(button.dataset.value || "");
      const original = button.textContent;
      button.textContent = "Copied";
      window.setTimeout(() => {
        button.textContent = original;
      }, 900);
    });
  }
}

function renderReferences(symbol) {
  const refs = referencesBySymbol.get(symbol.id) || [];
  const root = document.querySelector("#referenceList");
  if (refs.length === 0) {
    root.innerHTML = '<div class="empty">No references were exported for this symbol.</div>';
    return;
  }
  root.innerHTML = "";
  refs.slice(0, 200).forEach((reference, index) => {
    const source = sources.get(reference.sourceId);
    const button = document.createElement("button");
    button.type = "button";
    button.className = "reference-item";
    button.textContent = source.virtualPath + ":" + (reference.range.startLine + 1);
    button.addEventListener("click", async () => {
      state.sourceId = reference.sourceId;
      state.line = reference.range.startLine;
      setPanel("source");
      setHash("source=" + encodeURIComponent(source.virtualPath) + "&line=" + (reference.range.startLine + 1));
      await renderSource(reference.sourceId, reference.range.startLine, symbol.id, index);
    });
    root.append(button);
  });
}

async function renderSource(sourceId, focusLine, symbolId) {
  const sourceMeta = sources.get(sourceId);
  const sourceView = document.querySelector("#sourceView");
  if (!sourceMeta) {
    document.querySelector("#sourceTitle").textContent = "Source";
    document.querySelector("#sourceHint").textContent = "";
    sourceView.innerHTML = '<span class="state-message">No source selected.</span>';
    return;
  }
  document.querySelector("#sourceTitle").textContent = sourceMeta.virtualPath;
  document.querySelector("#sourceHint").textContent = sourceMeta.snapshot ? "snapshot" : "snippet or metadata";
  sourceView.innerHTML = '<span class="state-message">Loading source...</span>';
  try {
    const text = await loadSource(sourceId);
    const definitionLine = byId.get(symbolId)?.definition?.range?.startLine;
    const referenceLines = new Set((referencesBySymbol.get(symbolId) || [])
      .filter((ref) => ref.sourceId === sourceId)
      .map((ref) => ref.range.startLine));
    sourceView.innerHTML = text.split(/\r?\n/).map((line, index) => {
      const classes = ["line"];
      if (index === definitionLine) {
        classes.push("definition");
      }
      if (referenceLines.has(index)) {
        classes.push("reference");
      }
      return '<span class="' + classes.join(" ") + '" id="L' + (index + 1) + '"><span class="line-number">'
        + (index + 1) + '</span>' + escapeHtml(line) + '</span>';
    }).join("");
    if (typeof focusLine === "number") {
      document.querySelector("#L" + (focusLine + 1))?.scrollIntoView({ block: "center" });
    }
  } catch (error) {
    sourceView.innerHTML = '<span class="state-message error">' + escapeHtml(error.message || "Failed to load source.") + '</span>';
  }
}

function renderRecent() {
  const root = document.querySelector("#recentList");
  root.innerHTML = "";
  if (state.recent.length === 0) {
    root.innerHTML = '<div class="empty">No recent symbols.</div>';
    return;
  }
  for (const id of state.recent) {
    const symbol = byId.get(id);
    if (!symbol) {
      continue;
    }
    const button = document.createElement("button");
    button.type = "button";
    button.className = "recent-item";
    button.textContent = symbol.name;
    button.addEventListener("click", () => selectSymbol(id, true));
    root.append(button);
  }
}

function restoreFromHash() {
  const params = new URLSearchParams(location.hash.replace(/^#/, ""));
  const symbolName = params.get("symbol");
  if (symbolName) {
    const symbol = data.symbols.find((entry) => entry.name === symbolName);
    if (symbol) {
      selectSymbol(symbol.id, false);
    }
    return;
  }
  const sourcePath = params.get("source");
  if (sourcePath) {
    const source = data.sources.find((entry) => entry.virtualPath === sourcePath);
    const line = Math.max(0, Number(params.get("line") || "1") - 1);
    if (source) {
      setPanel("source");
      renderSource(source.id, line, state.symbolId);
    }
  }
}

function loadSource(id) {
  if (window.__SAGE_REFERENCE_SOURCES__[id]) {
    return Promise.resolve(window.__SAGE_REFERENCE_SOURCES__[id].text || "");
  }
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = "./data/sources/source-" + id + ".js";
    script.onload = () => resolve(window.__SAGE_REFERENCE_SOURCES__[id]?.text || "");
    script.onerror = () => reject(new Error("Failed to load source shard " + id));
    document.head.append(script);
  });
}

function moveReference(delta) {
  const refs = referencesBySymbol.get(state.symbolId) || [];
  if (!refs.length) {
    return;
  }
  const current = refs.findIndex((ref) => ref.sourceId === state.sourceId && ref.range.startLine === state.line);
  const next = refs[(Math.max(0, current) + delta + refs.length) % refs.length];
  const source = sources.get(next.sourceId);
  setPanel("source");
  setHash("source=" + encodeURIComponent(source.virtualPath) + "&line=" + (next.range.startLine + 1));
  renderSource(next.sourceId, next.range.startLine, state.symbolId);
}

function setHash(value) {
  history.replaceState(null, "", "#" + value);
}

function setPanel(panel) {
  document.documentElement.dataset.panel = panel;
}

function groupFor(symbol) {
  if (symbol.origin === "sage") {
    return "Sage API";
  }
  if (symbol.kind === "Module") {
    return "Modules";
  }
  if (["Class", "Struct", "Type"].includes(symbol.kind)) {
    return "Classes/Types";
  }
  if (["Function", "Method"].includes(symbol.kind)) {
    return "Functions";
  }
  return "Project";
}

function actionButton(label, value) {
  return '<button type="button" data-action="copy" data-value="' + escapeAttr(value) + '">' + escapeHtml(label) + '</button>';
}

function badge(value, warn) {
  return value ? '<span class="badge' + (warn ? " warn" : "") + '">' + escapeHtml(value) + '</span>' : "";
}

function fuzzy(text, needle) {
  return needle.split(/\s+/).every((part) => text.includes(part));
}

async function copyText(value) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(value);
  }
}

function formatGeneratedAt(value) {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function escapeHtml(value) {
  return String(value ?? "").replace(/[&<>"']/g, (char) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    '"': "&quot;",
    "'": "&#39;",
  }[char]));
}

function escapeAttr(value) {
  return escapeHtml(value);
}
