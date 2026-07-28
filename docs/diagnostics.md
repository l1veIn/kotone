# 诊断包与流程挖掘

Kotone 的诊断数据只保存在本机，不会自动上传。用户在「设置 → 关于」点击
「导出诊断包」后，才会生成可主动分享的 ZIP。

## 隐私边界

普通诊断包不包含：

- 录音；
- 识别文本；
- 热词内容；
- 窗口标题、进程列表；
- `config.json`、完整本机路径或下载代理地址。

包内 `history-metadata.json` 只保留识别耗时、结果、错误码、文本长度和是否曾保存
音频；不会复制历史原文或 WAV。`kotone.log` 是经过二次脱敏的副本，兼容清理升级
前曾写入日志的状态 payload 和用户主目录。

## 包内文件

| 文件 | 内容 |
| --- | --- |
| `manifest.json` | 报告编号、包格式版本、应用版本、隐私声明 |
| `environment.json` | Windows/内核版本、CPU 架构、是否提权 |
| `runtime.json` | 引擎、模型、热键、麦克风、Profile 与运行状态白名单 |
| `models.json` | 模型 ID、大小与就绪状态 |
| `history-metadata.json` | 最近 50 条识别历史的脱敏指标 |
| `events.csv` | 最近最多 20,000 条 PM4Py 兼容流程事件 |
| `kotone.log` | 最近 1 MB 脱敏运行日志 |

`events.csv` 的前三列遵循 PM4Py 约定：

- `case:concept:name`
- `concept:name`
- `time:timestamp`

其余列是引擎、模型、Profile、交互模式、结果、稳定错误码和耗时等低敏属性。

## 测试批次收集

遇到问题时，请用户把诊断包发给测试群管理员。为避免只收到失败用户的数据造成
选择偏差，每轮测试结束时还应让所有测试用户统一导出一次，无问题也提交。

诊断包可能包含一段重叠的本地事件历史；分析脚本会按 case、activity、timestamp
和 app session 去重。

## 离线 PM4Py 分析

安装分析依赖：

```powershell
python -m pip install pm4py
```

合并一个目录下的所有诊断包并生成流程图：

```powershell
python scripts/analyze_diagnostics.py C:\path\to\bundles --out .tmp\process-mining
```

只合并数据、不调用 PM4Py/Graphviz：

```powershell
python scripts/analyze_diagnostics.py C:\path\to\bundles --out .tmp\process-mining --merge-only
```

输出包括合并事件、路径变体、汇总、直接跟随图和归纳流程树。PM4Py 只作为开发侧
离线工具使用，不打包进 Kotone，也不会产生网络上传。
