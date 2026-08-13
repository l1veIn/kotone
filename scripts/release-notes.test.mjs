import assert from "node:assert/strict";
import test from "node:test";
import {
  findReleaseSection,
  findUnreleasedSection,
  prepareChangelogVersion,
  releaseHeadingPattern,
  requireReleaseNotes,
} from "./release-notes.mjs";

test("matches stable and prerelease headings exactly", () => {
  assert.match("## 0.1.8 — 2026-08-13", releaseHeadingPattern("0.1.8"));
  assert.match("## 0.1.8-rc.1", releaseHeadingPattern("0.1.8-rc.1"));
  assert.doesNotMatch("## 0.1.80 — 2026-08-13", releaseHeadingPattern("0.1.8"));
});

test("extracts only the requested CRLF release section", () => {
  const changelog = [
    "# Changelog",
    "",
    "## 0.1.8 — 2026-08-13",
    "",
    "- 第一项",
    "- 第二项",
    "",
    "## 0.1.7 — 2026-08-12",
    "",
    "旧版",
  ].join("\r\n");
  assert.equal(requireReleaseNotes(changelog, "0.1.8"), "- 第一项\n- 第二项");
});

test("rejects missing, empty and placeholder release notes", () => {
  assert.throws(() => requireReleaseNotes("# Changelog\n", "0.1.8"), /找不到/);
  assert.throws(() => requireReleaseNotes("# Changelog\n\n## 0.1.8\n", "0.1.8"), /没有更新说明/);
  assert.throws(
    () => requireReleaseNotes("# Changelog\n\n## 0.1.8\n\n（待补充：摘要）\n", "0.1.8"),
    /占位/,
  );
});

test("finds version and Unreleased sections independently", () => {
  const changelog = "# Changelog\n\n## Unreleased\n\n- 新功能\n\n## 0.1.7 — 2026-08-12\n\n- 旧功能\n";
  assert.equal(findUnreleasedSection(changelog)?.content, "- 新功能");
  assert.equal(findReleaseSection(changelog, "0.1.7")?.content, "- 旧功能");
});

test("archives Unreleased and preserves CRLF", () => {
  const changelog = "# Changelog\r\n\r\n## Unreleased\r\n\r\n- 新功能\r\n\r\n## 0.1.7 — 2026-08-12\r\n\r\n- 旧功能\r\n";
  const result = prepareChangelogVersion(changelog, "0.1.8", "2026-08-13");
  assert.equal(result.kind, "archived");
  assert.match(result.text, /^# Changelog\r\n\r\n## Unreleased\r\n\r\n## 0\.1\.8 — 2026-08-13\r\n/);
  assert.equal(requireReleaseNotes(result.text, "0.1.8"), "- 新功能");
});

test("rejects an existing target while Unreleased still has content", () => {
  const changelog = "# Changelog\n\n## Unreleased\n\n- 未归档\n\n## 0.1.8 — 2026-08-13\n\n- 已归档\n";
  assert.throws(
    () => prepareChangelogVersion(changelog, "0.1.8", "2026-08-13"),
    /Unreleased 仍有内容/,
  );
});
