import { expect, test } from "bun:test";
import schema from "../schema/batman.schema.json" with { type: "json" };
import type { InitializeParams } from "./generated/InitializeParams";

test("schema is draft 2020-12", () => {
  expect(schema.$schema).toBe("https://json-schema.org/draft/2020-12/schema");
});

test("generated type accepts the golden initialize request", async () => {
  const value = (await Bun.file("fixtures/protocol/initialize.request.json").json()) as InitializeParams;
  expect(value.client.name).toBe("@nikolasd/batman");
});
