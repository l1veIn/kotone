#!/usr/bin/env python3
"""Merge Kotone diagnostic bundles and run offline PM4Py analysis.

The script never uploads data. It reads `events.csv` from one or more diagnostic
ZIPs, de-duplicates overlapping exports, writes a merged CSV/variant summary,
and optionally renders a directly-follows graph plus an inductive process tree.
"""

from __future__ import annotations

import argparse
import csv
import json
import sys
import zipfile
from collections import Counter, defaultdict
from pathlib import Path
from typing import Iterable

CORE_COLUMNS = (
    "case:concept:name",
    "concept:name",
    "time:timestamp",
    "eventIndex",
    "appSessionId",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="合并 Kotone 诊断包并生成 PM4Py 流程挖掘结果"
    )
    parser.add_argument(
        "inputs",
        nargs="+",
        type=Path,
        help="诊断 ZIP，或包含诊断 ZIP 的目录（目录会递归扫描）",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=Path("process-mining-output"),
        help="输出目录（默认 process-mining-output）",
    )
    parser.add_argument(
        "--merge-only",
        action="store_true",
        help="只合并 CSV 和统计变体，不调用 PM4Py/Graphviz",
    )
    return parser.parse_args()


def discover_bundles(inputs: Iterable[Path]) -> list[Path]:
    bundles: set[Path] = set()
    for item in inputs:
        if item.is_dir():
            bundles.update(path for path in item.rglob("*.zip") if path.is_file())
        elif item.is_file() and item.suffix.lower() == ".zip":
            bundles.add(item)
    return sorted(bundles)


def load_bundle(path: Path) -> tuple[str, list[dict[str, str]]]:
    with zipfile.ZipFile(path) as archive:
        if "events.csv" not in archive.namelist():
            return path.stem, []
        report_id = path.stem
        if "manifest.json" in archive.namelist():
            try:
                manifest = json.loads(archive.read("manifest.json").decode("utf-8"))
                report_id = str(manifest.get("reportId") or report_id)
            except (UnicodeDecodeError, json.JSONDecodeError):
                pass
        text = archive.read("events.csv").decode("utf-8-sig")
        rows = list(csv.DictReader(text.splitlines()))
        for row in rows:
            row["sourceBundle"] = path.name
            row["reportId"] = report_id
            # 同一诊断包内，appSessionId + 原 case id 是稳定且更抗碰撞的 case。
            app_session = row.get("appSessionId", "")
            original_case = row.get("case:concept:name", "")
            row["case:concept:name"] = f"{app_session}/{original_case}"
        return report_id, rows


def deduplicate(rows: Iterable[dict[str, str]]) -> list[dict[str, str]]:
    seen: set[tuple[str, ...]] = set()
    unique: list[dict[str, str]] = []
    for row in rows:
        key = tuple(row.get(column, "") for column in CORE_COLUMNS)
        if key in seen:
            continue
        seen.add(key)
        unique.append(row)
    unique.sort(
        key=lambda row: (
            row.get("case:concept:name", ""),
            row.get("time:timestamp", ""),
            row.get("concept:name", ""),
        )
    )
    return unique


def write_merged_csv(path: Path, rows: list[dict[str, str]]) -> None:
    preferred = [
        "case:concept:name",
        "concept:name",
        "time:timestamp",
        "eventIndex",
        "appSessionId",
        "appVersion",
        "engineId",
        "modelId",
        "profileId",
        "interactionMode",
        "elevated",
        "outcome",
        "errorCode",
        "durationMs",
        "audioMs",
        "textChars",
        "reportId",
        "sourceBundle",
    ]
    extras = sorted({key for row in rows for key in row} - set(preferred))
    with path.open("w", encoding="utf-8-sig", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=preferred + extras)
        writer.writeheader()
        writer.writerows(rows)


def write_variants(path: Path, rows: list[dict[str, str]]) -> None:
    cases: dict[str, list[tuple[str, str]]] = defaultdict(list)
    for row in rows:
        cases[row.get("case:concept:name", "")].append(
            (
                row.get("time:timestamp", ""),
                int(row.get("eventIndex", "0") or 0),
                row.get("concept:name", ""),
            )
        )
    variants = Counter(
        " → ".join(activity for _, _, activity in sorted(events))
        for events in cases.values()
        if events
    )
    with path.open("w", encoding="utf-8-sig", newline="") as handle:
        writer = csv.writer(handle)
        writer.writerow(["count", "variant"])
        writer.writerows(variants.most_common())


def run_pm4py(csv_path: Path, output_dir: Path) -> None:
    try:
        import pandas as pd
        import pm4py
    except ImportError as error:
        raise RuntimeError(
            "缺少 PM4Py。请运行 `python -m pip install pm4py`，"
            "或加 --merge-only 只生成合并数据。"
        ) from error

    frame = pd.read_csv(csv_path)
    frame = frame.sort_values(
        ["case:concept:name", "time:timestamp", "eventIndex"], kind="stable"
    )
    frame = pm4py.format_dataframe(
        frame,
        case_id="case:concept:name",
        activity_key="concept:name",
        timestamp_key="time:timestamp",
    )
    dfg, starts, ends = pm4py.discover_dfg(frame)
    pm4py.save_vis_dfg(dfg, starts, ends, str(output_dir / "directly-follows.svg"))

    process_tree = pm4py.discover_process_tree_inductive(frame)
    pm4py.save_vis_process_tree(process_tree, str(output_dir / "process-tree.svg"))


def main() -> int:
    args = parse_args()
    bundles = discover_bundles(args.inputs)
    if not bundles:
        print("没有找到包含诊断数据的 ZIP。", file=sys.stderr)
        return 2

    all_rows: list[dict[str, str]] = []
    used_bundles = 0
    for bundle in bundles:
        _, rows = load_bundle(bundle)
        if rows:
            used_bundles += 1
            all_rows.extend(rows)
    rows = deduplicate(all_rows)
    if not rows:
        print("诊断包中没有 events.csv 事件。", file=sys.stderr)
        return 2

    args.out.mkdir(parents=True, exist_ok=True)
    merged_csv = args.out / "merged-events.csv"
    write_merged_csv(merged_csv, rows)
    write_variants(args.out / "variants.csv", rows)
    summary = {
        "bundles": used_bundles,
        "events": len(rows),
        "cases": len({row.get("case:concept:name", "") for row in rows}),
    }
    (args.out / "summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8"
    )

    if not args.merge_only:
        try:
            run_pm4py(merged_csv, args.out)
        except (RuntimeError, OSError) as error:
            print(f"PM4Py 可视化未完成：{error}", file=sys.stderr)
            print(f"合并数据已保留在 {merged_csv}", file=sys.stderr)
            return 1

    print(
        f"完成：{used_bundles} 个诊断包，{summary['cases']} 个 case，"
        f"{summary['events']} 条事件 → {args.out}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
