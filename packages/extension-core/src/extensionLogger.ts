import * as vscode from "vscode";
import type { LoggingLevel } from "./settingsModel";

const LEVEL_WEIGHT: Record<LoggingLevel, number> = {
  error: 0,
  warn: 1,
  info: 2,
  debug: 3,
};

export class OutputLogger {
  constructor(
    private readonly outputChannel: vscode.OutputChannel,
    private level: LoggingLevel = "info",
  ) {}

  setLevel(level: LoggingLevel): void {
    this.level = level;
  }

  error(component: string, message: string, fields: Record<string, unknown> = {}): void {
    this.log("error", component, message, fields);
  }

  warn(component: string, message: string, fields: Record<string, unknown> = {}): void {
    this.log("warn", component, message, fields);
  }

  info(component: string, message: string, fields: Record<string, unknown> = {}): void {
    this.log("info", component, message, fields);
  }

  debug(component: string, message: string, fields: Record<string, unknown> = {}): void {
    this.log("debug", component, message, fields);
  }

  log(
    level: LoggingLevel,
    component: string,
    message: string,
    fields: Record<string, unknown> = {},
  ): void {
    if (LEVEL_WEIGHT[level] > LEVEL_WEIGHT[this.level]) {
      return;
    }
    this.outputChannel.appendLine(formatLogLine(level, component, message, fields));
  }
}

export function createOutputLogger(
  outputChannel: vscode.OutputChannel,
  level: LoggingLevel = "info",
): OutputLogger {
  return new OutputLogger(outputChannel, level);
}

export function logToChannel(
  outputChannel: vscode.OutputChannel,
  configuredLevel: LoggingLevel,
  level: LoggingLevel,
  component: string,
  message: string,
  fields: Record<string, unknown> = {},
): void {
  if (LEVEL_WEIGHT[level] > LEVEL_WEIGHT[configuredLevel]) {
    return;
  }
  outputChannel.appendLine(formatLogLine(level, component, message, fields));
}

export function formatLogLine(
  level: LoggingLevel,
  component: string,
  message: string,
  fields: Record<string, unknown> = {},
): string {
  const suffix = Object.entries(fields)
    .filter(([, value]) => value !== undefined && value !== null)
    .map(([key, value]) => `${key}=${formatFieldValue(value)}`)
    .join(" ");
  return `[${level}] [${component}] ${message}${suffix ? ` ${suffix}` : ""}`;
}

function formatFieldValue(value: unknown): string {
  const text = String(value);
  return /\s/.test(text) ? JSON.stringify(text) : text;
}
