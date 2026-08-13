/** CHANGELOG 二级标题中的版本号必须完整匹配，避免 0.1.8 误命中 0.1.80。 */
export function releaseHeadingPattern(version) {
  const escaped = version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`^##\\s+${escaped}(?:\\s+—(?:\\s+.*)?)?\\s*$`);
}

export function findReleaseSection(changelog, version) {
  const lines = changelog.split(/\r?\n/);
  const start = lines.findIndex((line) => releaseHeadingPattern(version).test(line));
  if (start < 0) return null;
  const nextHeading = lines.slice(start + 1).findIndex((line) => /^##\s+/.test(line));
  const end = nextHeading < 0 ? lines.length : start + 1 + nextHeading;
  return {
    lines,
    start,
    end,
    content: lines.slice(start + 1, end).join("\n").trim(),
  };
}

export function findUnreleasedSection(changelog) {
  const lines = changelog.split(/\r?\n/);
  const start = lines.findIndex((line) => /^##\s+Unreleased\s*$/i.test(line));
  if (start < 0) return null;
  const nextHeading = lines.slice(start + 1).findIndex((line) => /^##\s+/.test(line));
  const end = nextHeading < 0 ? lines.length : start + 1 + nextHeading;
  return {
    lines,
    start,
    end,
    content: lines.slice(start + 1, end).join("\n").trim(),
  };
}

export function requireReleaseNotes(changelog, version) {
  const section = findReleaseSection(changelog, version);
  if (!section) throw new Error(`CHANGELOG 中找不到版本 ${version} 的二级标题。`);
  if (!section.content) throw new Error(`CHANGELOG 的 ${version} 小节没有更新说明。`);
  if (/待补充|\b(?:TODO|TBD)\b/i.test(section.content)) {
    throw new Error(`CHANGELOG 的 ${version} 小节仍含待补充占位内容。`);
  }
  return section.content;
}

export function prepareChangelogVersion(changelog, version, date) {
  if (!changelog.startsWith("# Changelog")) {
    throw new Error("CHANGELOG.md 开头不是 '# Changelog'。");
  }
  const target = findReleaseSection(changelog, version);
  const unreleased = findUnreleasedSection(changelog);
  if (target) {
    if (unreleased?.content) {
      throw new Error(`CHANGELOG.md 已有 ${version} 小节，但 Unreleased 仍有内容。`);
    }
    return { kind: "unchanged", text: changelog };
  }

  const eol = changelog.includes("\r\n") ? "\r\n" : "\n";
  const lines = changelog.split(/\r?\n/);
  const headerLength = lines[1]?.trim() === "" ? 2 : 1;
  if (unreleased) {
    lines[unreleased.start] = `## ${version} — ${date}`;
    lines.splice(headerLength, 0, "## Unreleased", "");
    return { kind: "archived", text: lines.join(eol) };
  }

  const block = [`## ${version} — ${date}`, "", "（待补充：本次发布变更摘要）", ""];
  return {
    kind: "inserted",
    text: [...lines.slice(0, headerLength), ...block, ...lines.slice(headerLength)].join(eol),
  };
}
