// Conformance report completeness validator.
//
// Validates that a conformance report contains all expected adapters and
// that each adapter's report has the required fields.

import { readFileSync } from "node:fs";
// Conformance report completeness validator.
//
// STUB: Validates that a conformance report contains all expected adapters and
// that each adapter's report has the required fields. Does NOT verify that
// scenarios actually ran or passed — only checks field presence.

import { readFileSync } from "node:fs";

// Conformance report types (mirror of Rust struct)
interface ScenarioResult {
  readonly scenario: string;
  readonly status: "passed" | "failed" | "skipped";
  readonly duration_ms?: number;
  readonly error?: string;
}

interface ConformanceReport {
  readonly adapter: string;
  readonly mode: "fixture" | "live";
  readonly declaredCapabilities: string[];
  readonly effectiveCapabilities: string[];
  readonly scenarios: ScenarioResult[];
}
/**
 * Loads and validates a conformance report file.
 *
 * @param filePath - Path to the conformance report JSON file
 * @throws Error if the report is missing required fields or adapters
 */
export function loadAndValidateReport(filePath: string): void {
  let content: string;
  try {
    content = readFileSync(filePath, "utf-8");
  } catch (err) {
    throw new Error(`Failed to read report file ${filePath}: ${err}`);
  }

  let report: unknown;
  try {
    report = JSON.parse(content);
  } catch (err) {
    throw new Error(`Failed to parse report JSON: ${err}`);
  }

  assertReportComplete(report);
}

/**
 * Asserts that a conformance report is complete (all expected adapters present).
 */
export function assertReportComplete(report: unknown): void {
  if (!report || typeof report !== "object") {
    throw new Error("Report must be an object");
  }

  const r = report as { adapters?: Record<string, ConformanceReport> };
  if (!r.adapters) {
    throw new Error("Report missing 'adapters' field");
  }

  const expectedAdapters = ["claude", "codex", "copilot", "omp-rpc"];
  for (const adapter of expectedAdapters) {
    if (!r.adapters[adapter]) {
      throw new Error(`Report missing adapter: ${adapter}`);
    }

    const adapterReport = r.adapters[adapter];
    assertAdapterReportValid(adapterReport, adapter);
  }
}

/**
 * Asserts that an individual adapter's conformance report is valid.
 */
function assertAdapterReportValid(report: ConformanceReport, adapterName: string): void {
  if (report.adapter !== adapterName) {
    throw new Error(`Adapter mismatch: report says '${report.adapter}', expected '${adapterName}'`);
  }

  if (!["fixture", "live"].includes(report.mode)) {
    throw new Error(`Adapter ${adapterName} has invalid mode: '${report.mode}'`);
  }

  if (!Array.isArray(report.scenarios)) {
    throw new Error(`Adapter ${adapterName} missing 'scenarios' array`);
  }

  if (!Array.isArray(report.declaredCapabilities)) {
    throw new Error(`Adapter ${adapterName} missing 'declaredCapabilities' array`);
  }

  if (!Array.isArray(report.effectiveCapabilities)) {
    throw new Error(`Adapter ${adapterName} missing 'effectiveCapabilities' array`);
  }
}
