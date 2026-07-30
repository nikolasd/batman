/**
 * Conformance gates for the BATMAN runtime.
 *
 * This module provides a TypeScript runner for conformance tests,
 * verifying that the runtime behaves correctly across all supported
 * adapter kinds (claude, codex, copilot, ompRpc).
 *
 * Conformance tests are divided into two modes:
 * - Fixture mode: Zero-model-call tests that verify structural correctness
 * - Live mode: Real model-call tests that verify end-to-end behavior
 *
 * Each test produces a machine-readable report suitable for CI integration.
 */

import { execSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

/**
 * The adapter kind being tested.
 */
export type AdapterKind = 'claude' | 'codex' | 'copilot' | 'ompRpc' | 'all';

/**
 * The conformance test mode.
 */
export type ConformanceMode = 'fixture' | 'live';

/**
 * Configuration for running conformance tests.
 */
export interface ConformanceConfig {
  /** The adapter kind to test (or 'all' for all adapters). */
  adapter: AdapterKind;

  /** The conformance test mode. */
  mode: ConformanceMode;

  /** The output file path for the report (defaults to stdout). */
  outputFile?: string;

  /** The BATMAN state root directory. */
  stateDir: string;

  /** The repository path to test against. */
  repo: string;
}

/**
 * A single conformance test result.
 */
export interface ConformanceTestResult {
  /** The adapter kind being tested. */
  adapter: string;

  /** The test name. */
  testName: string;

  /** Whether the test passed. */
  passed: boolean;

  /** An optional error message if the test failed. */
  error?: string;

  /** The time taken to run the test in milliseconds. */
  duration: number;
}

/**
 * A complete conformance report for one or more adapters.
 */
export interface ConformanceReport {
  /** The adapter kind this report is for. */
  adapter: string;

  /** The conformance mode (fixture or live). */
  mode: ConformanceMode;

  /** The timestamp when the report was generated. */
  timestamp: string;

  /** The set of test results. */
  tests: ConformanceTestResult[];

  /** Whether all tests passed. */
  allPassed: boolean;
}

/**
 * Runs conformance tests for the specified adapter kind and mode.
 *
 * @param config - The conformance test configuration.
 * @returns The conformance report.
 *
 * @example
 * ```typescript
 * const report = await runConformance({
 *   adapter: 'claude',
 *   mode: 'fixture',
 *   stateDir: '/tmp/bat-state',
 *   repo: '/path/to/repo',
 * });
 * console.log(report.allPassed ? 'All tests passed' : 'Some tests failed');
 * ```
 */
export async function runConformance(config: ConformanceConfig): Promise<ConformanceReport> {
  const tests: ConformanceTestResult[] = [];
  const startTime = Date.now();

  // Determine which adapters to test
  const adapters = config.adapter === 'all'
    ? ['claude', 'codex', 'copilot', 'ompRpc']
    : [config.adapter];

  for (const adapter of adapters) {
    try {
      // Run the conformance test via the batcave CLI
      const result = execSync(
        `batcave conformance --adapter ${adapter} --${config.mode} --state-dir ${config.stateDir} --repo ${config.repo}`,
        {
          encoding: 'utf-8',
          timeout: 60000, // 1 minute timeout
          stdio: ['pipe', 'pipe', 'pipe'],
        }
      );

      // Parse the JSONL output
      const lines = result.trim().split('\n').filter(Boolean);
      for (const line of lines) {
        try {
          const testResult = JSON.parse(line) as ConformanceTestResult;
          tests.push(testResult);
        } catch (parseError) {
          // If we can't parse a line, treat it as a test failure
          tests.push({
            adapter,
            testName: 'unknown',
            passed: false,
            error: `Failed to parse test output: ${line}`,
            duration: 0,
          });
        }
      }
    } catch (error) {
      // If the command fails, record it as a test failure
      const errorMessage = error instanceof Error ? error.message : String(error);
      tests.push({
        adapter,
        testName: `${adapter}-${config.mode}`,
        passed: false,
        error: errorMessage,
        duration: Date.now() - startTime,
      });
    }
  }

  const report: ConformanceReport = {
    adapter: config.adapter,
    mode: config.mode,
    timestamp: new Date().toISOString(),
    tests,
    allPassed: tests.every(t => t.passed),
  };

  // Write report to file if output file is specified
  if (config.outputFile) {
    const outputDir = path.dirname(config.outputFile);
    if (!fs.existsSync(outputDir)) {
      fs.mkdirSync(outputDir, { recursive: true });
    }
    fs.writeFileSync(config.outputFile, JSON.stringify(report, null, 2));
  }

  return report;
}

/**
 * Prints a human-readable summary of the conformance report.
 *
 * @param report - The conformance report to summarize.
 * @returns A human-readable summary string.
 *
 * @example
 * ```typescript
 * const report = await runConformance(config);
 * console.log(formatConformanceSummary(report));
 * ```
 */
export function formatConformanceSummary(report: ConformanceReport): string {
  const lines: string[] = [];

  lines.push(`Conformance Report`);
  lines.push(`==================`);
  lines.push(`Adapter: ${report.adapter}`);
  lines.push(`Mode: ${report.mode}`);
  lines.push(`Timestamp: ${report.timestamp}`);
  lines.push(`All Passed: ${report.allPassed ? 'Yes' : 'No'}`);
  lines.push(``);
  lines.push(`Tests:`);

  for (const test of report.tests) {
    const status = test.passed ? '✓' : '✗';
    lines.push(`  ${status} ${test.testName} (${test.duration}ms)`);
    if (test.error) {
      lines.push(`    Error: ${test.error}`);
    }
  }

  return lines.join('\n');
}
