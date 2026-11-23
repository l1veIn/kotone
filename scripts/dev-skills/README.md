# 开发者自用的技能

这里的技能**不打包进安装包**，普通玩家看不到。开发者和测试玩家自己装到 Kotone 里用：「设置 → 琴音 → 技能 → 导入」，选对应的文件夹。

| 技能 | 做什么 | 需要 |
| --- | --- | --- |
| `kotone-issue` | 把用 Kotone 时遇到的问题整理成 GitHub issue，提交到 `l1veIn/kotone`（先给你看草稿，你说可以才提交） | 本机装了 GitHub CLI（`winget install GitHub.cli`）并运行过 `gh auth login` |

改了技能以后重新导入一次（同名的会覆盖，旧版本留在 `skills/.versions/` 里）。

真机测试：`python -X utf8 -I scripts/manual-e2e/kotone_issue_real.py`（隔离的 KOTONE_HOME、真 DeepSeek Key，会在公开仓库建一个测试 issue，最后关掉）。
