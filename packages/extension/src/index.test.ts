import { expect, test } from "bun:test";
import extension from "./index";

test("exports an OMP extension factory", () => {
  expect(typeof extension).toBe("function");
});
