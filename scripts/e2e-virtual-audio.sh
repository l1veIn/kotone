#!/usr/bin/env bash
# Kotone 无人值守音频自动化 E2E（docs/cli.md 配套脚本）
#
#   路径 1：wav 直灌——listen --wav（WavFileBackend，不经过系统音频栈）
#   路径 2：虚拟声卡回路——play → CABLE Input → CABLE Output → listen --no-hotkey
#
# 用法（仓库根目录的 Git Bash 中）：scripts/e2e-virtual-audio.sh
# 断言：两条路径产出的 JSONL 中 final 文本 = SAPI fixture 原文「对面打野在下路」
#
# 隔离的 KOTONE_HOME（target/cli-e2e/home）：不读写玩家的 ~/.kotone（配置、识别历史）；模型目录指向
# ~/.kotone/models（只读，要先装好流式模型 x-asr，或设 KOTONE_E2E_MODELS 指向别处）。
set -u
cd "$(dirname "$0")/.."

CLI="cargo run -q -p kotone-cli --"
FIXTURE="crates/kotone-stt/tests/fixtures/zh-game-3s.wav"
EXPECT="对面打野在下路"
ENGINE="sherpa-onnx-x-asr-zh-en"
TMP=".tmp-e2e"
mkdir -p "$TMP"
fail=0

HOME_DIR="target/cli-e2e/home"
MODELS="${KOTONE_E2E_MODELS:-$HOME/.kotone/models}"
rm -rf "$HOME_DIR" && mkdir -p "$HOME_DIR"
printf '{"models":{"dir":"%s"},"ui":{"firstRunCompleted":true}}' "$(cygpath -m "$MODELS" 2>/dev/null || echo "$MODELS")" > "$HOME_DIR/config.json"
export KOTONE_HOME="$(cygpath -w "$PWD/$HOME_DIR" 2>/dev/null || echo "$PWD/$HOME_DIR")"

echo "== 构建 kotone-cli =="
cargo build -p kotone-cli || exit 1

echo ""
echo "== 路径 1：wav 直灌（listen --wav，不依赖任何音频设备）=="
$CLI listen --wav "$FIXTURE" --engine "$ENGINE" > "$TMP/wav-direct.jsonl" 2>&1
code=$?
echo "退出码: $code（0=成功）"
if grep -qF "\"text\":\"$EXPECT\"" "$TMP/wav-direct.jsonl" && [ $code -eq 0 ]; then
    echo "路径 1 PASS：final 文本匹配「$EXPECT」"
else
    echo "路径 1 FAIL（JSONL 见 $TMP/wav-direct.jsonl）"
    fail=1
fi

echo ""
echo "== 路径 2：虚拟声卡回路（play → CABLE Input → CABLE Output → listen）=="
devices=$($CLI devices 2>/dev/null)
cable_capture=$(echo "$devices" | grep '^IN' | grep -i 'cable' | head -1 | cut -d'|' -f2 | sed 's/^ *//;s/ *$//')
if [ -z "$cable_capture" ]; then
    echo "未检测到虚拟声卡（VB-CABLE 类），跳过路径 2"
    echo "安装 VB-CABLE 后重跑本脚本即可启用该路径"
else
    echo "采集设备: $cable_capture"
    # 改的是隔离的配置（KOTONE_HOME），不碰玩家的
    $CLI config set audioDeviceId "$cable_capture" || fail=1
    $CLI config set autoSend false || fail=1

    $CLI listen --no-hotkey --duration 6 --engine "$ENGINE" > "$TMP/cable.jsonl" 2>&1 &
    listen_pid=$!
    sleep 1.5   # 等采集流打开
    $CLI play "$FIXTURE" --device "CABLE Input" || fail=1
    wait $listen_pid
    code=$?
    echo "退出码: $code（0=成功）"

    # 经声卡回路时第一个字可能被吞（“对面”→“免”），只查后半句；listen 不带 --send：只识别到预览，不往前台窗口打字
    if grep -qF "打野在下路" "$TMP/cable.jsonl" && grep -qF '"state":"preview"' "$TMP/cable.jsonl"         && ! grep -qF '"state":"sending"' "$TMP/cable.jsonl" && [ $code -eq 0 ]; then
        echo "路径 2 PASS：经虚拟声卡回路识别出「…打野在下路」，没有打字"
    else
        echo "路径 2 FAIL（JSONL 见 $TMP/cable.jsonl）"
        fail=1
    fi
fi

echo ""
if [ $fail -eq 0 ]; then
    echo "E2E 全部通过"
else
    echo "E2E 存在失败"
fi
exit $fail
