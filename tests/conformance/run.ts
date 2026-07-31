// Aggregate conformance test runner.
//
// Runs all adapter fixture conformance reports and writes a combined report
// to the specified output path. Used by CI and manual testing.

import { writeFileSync } from "node:fs";

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

interface CombinedReport {
  readonly timestamp: string;
  readonly adapters: Record<string, ConformanceReport>;
}

/**
 * Runs all adapter fixture conformance reports and writes a combined report.
 *
 * STUB: This is a placeholder implementation. The real implementation would
 * spawn `batcave conformance --adapter <name> --output <path>` for each
 * adapter and combine the results. Currently writes empty reports for all
 * adapters.
 *
 * @param outputPath - Path to write the combined report JSON
 */
export async function runAllFixtures(outputPath: string): Promise<void> {
  const adapters = ["claude", "codex", "copilot", "omp-rpc"] as const;
  const combined: Record<string, ConformanceReport> = {};

  for (const adapter of adapters) {
    // STUB: in real implementation, spawn batcave conformance command
    combined[adapter] = {
      adapter,
      mode: "fixture",
      declaredCapabilities: [],
      effectiveCapabilities: [],
      scenarios: [],
    };
  }

  const report: CombinedReport = {
    timestamp: new Date().toISOString(),
    adapters: combined,
  };

  writeFileSync(outputPath, JSON.stringify(report, null, 2));
}

/**
 * Asserts that a conformance report is complete (all expected adapters present).
 *
 * STUB: Only checks field presence, not that scenarios actually ran or passed.
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
  }
}
