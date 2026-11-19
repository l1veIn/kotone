import assert from "node:assert/strict";
import test from "node:test";
import { transitionManifest, transitionNotes } from "./stage-transition-update.mjs";

const signed = {
  version: "0.2.0",
  notes: "Full notes",
  platforms: { "windows-x86_64-nsis": { url: "https://example.invalid/Kotone.exe", signature: "fixture" } },
};

test("adds local rollout percentages without changing the package metadata", () => {
  for (const percent of [0, 10, 50, 100]) {
    const manifest = transitionManifest(signed, percent);
    assert.equal(manifest.rolloutPercent, percent);
    assert.equal(manifest.notes, transitionNotes);
    assert.equal(manifest.notes.split("\n").length, 4);
    assert.deepEqual(manifest.platforms, signed.platforms);
  }
  assert.equal(signed.notes, "Full notes");
});

test("rejects unsafe percentages, release candidates, and missing signature metadata", () => {
  for (const value of [undefined, "10", -1, 0.5, 101]) {
    assert.throws(() => transitionManifest(signed, value));
  }
  assert.throws(() => transitionManifest({ ...signed, version: "0.2.0-rc.1" }, 10));
  assert.throws(() => transitionManifest({ version: "0.2.0", platforms: {} }, 10));
});
