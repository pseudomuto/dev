import assert from "node:assert/strict";
import { test } from "node:test";

import { greeting } from "./main.ts";

test("greeting", () => {
  assert.equal(greeting("{{name}}"), "Hello from {{name}}!");
});
