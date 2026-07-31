# Kotone 官网 · Direction C「电竞记分牌 Scoreboard」设计说明

> 同一内容骨架、第二套视觉方向。Gate 1 母版 C 已生成并存档
> （`ord-web-hero-scoreboard`，delivered），本页基于该母版的 HUD 语言做
> html-first 逐区实现。
>
> Gate 2 自动 QA 已通过（Astro 构建 ✓ 双 locale ✓ 无未定义渲染 ✓
> 资源引用 ✓），人工视觉验收待定。

## 设计来源

- **Hero 母版**：`ord-web-hero-scoreboard`（Kotone 选手卡居右 + 记分板面板居左）
- 复用生产资产：`ord-app-kotone-cutout-v2`（透明 cutout）、真实截图、
  switch-cross 共享纹理（live CSS）
- 内容骨架与 i18n 自洽（本目录即官网，同源 locale 文件）

## 视觉语法

- **切角面板**：`clip-path` 缺角（`--cut: 14px`）替代圆角，广播转播风格
- **HUD 语言**：◤◢ 角标、`[ ]` 导航括号、扫描线纹理、switch-cross 网格线
- **分区命名**：为什么→对决面板 / 功能→LOADOUT / 上手→OBJECTIVE /
  界面→REPLAYS / 隐私→SERVER RULES / CTA→PRESS START / 页脚→谢幕字幕
- **颜色纪律**：青=边框/刻度/读数，品红=CTA/高亮，紫=过渡，深蓝黑基底
- 数据徽章做成 HUD 读数框（顶部青色刻度线 + 发光数值）

## Bake Mask（与 A 相同骨架，视觉换皮）

| Section | 设计来源 | baked / live | 共享 L1 |
|---|---|---|---|
| Nav | html-first | 全 live | HUD 顶栏，滚动后青色下边线 |
| Hero | `ord-web-hero-scoreboard` | baked=[L1,L2]，live=[L3,L4] | switch-cross 网格 + 扫描线 |
| Why | html-first | 全 live | 网格线 |
| Features | html-first | live=[L3,L4]，L2 内联 SVG | 网格线 + 扫描线 |
| Workflow | html-first + cutout 客串 | live=[L3,L4]，L2 cutout | 网格线 |
| Proof | html-first | 真实截图 live | 网格线 |
| Privacy | html-first | 全 live | 网格线 |
| CTA | html-first（氛围 + 网格线） | live=[L3,L4] | 网格线 |
| Footer | html-first | 全 live | 网格线 |

## 角色增强（第二轮）

新生成并已抠图（`character-cutout` 模板 + chroma-key 96/34）：

| 资产 | 订单 | 用途 |
|---|---|---|
| CTA 胜利庆祝（竖大拇指） | `ord-web-cta-cutout` | CTA 大角色 +「收到，已发送 ✨」live 气泡 |
| 无奈被搞心态反应 | `ord-web-reaction-pain` | 「以前的你」面板探出表情 |
| 得意比 V 反应 | `ord-web-reaction-win` | 「现在的你」面板探出表情 |

复用切片：`ord-sticker-001` 3×3 贴纸 → 透明 chibi →
导航小头像、Hero 漂浮装饰 ×3、页脚签名。

## 动效（Direction C 专属，全部 reduced-motion 降级）

- **Hero CRT**：扫描线慢速下滚 + 每 7–14s 随机 TV 故障爆发
  （RGB 分离 / 横向切片跳变 / 噪点闪烁 / 信号线扫过 / 内容跳切，JS 调度）
- **滚动进场**：全站 `[data-reveal]` IntersectionObserver，卡片/步骤错峰 `--rd`
- **Hero 入场级联**：eyebrow→标题→链路→说明→CTA→读数 错峰 riseIn
- **信号链路**：5 节点按序扫描发光（`--i` 延迟），发送节点品红脉冲
- **微交互**：按钮 sheen 扫光、功能图标 hover 旋转、截图 hover 缩放、
  记分牌菱形呼吸、CTA 角色上下漂浮 + 气泡弹跳
- **导航 scroll-spy**：当前区块高亮 `[ ]` 括号

## 第三轮修订（用户反馈）

- **Hero 扫描线**：去掉细微 9s 滚动 → 保留静态细条纹 + 新增 1–2 条**亮扫描线**
  （青色主扫线每 9s 扫一次、品红副扫线延迟 4.2s），老旧电视机感
- **Hero 贴纸装饰移除**：`hero__decals` 三枚 chibi 去除
- **Why 区重排**：文字卡片集中左侧，右侧单独站两个 cutout
  （无奈灰暗 ↔ 得意发光，一弱一强对比）
- **三步上手 → 四种交互模式**：对讲机 / 录音笔 / 说一句就走 / 独奏模式
  （源自应用内 `interaction.rs` 与设置页 `modes`），独奏模式做「精髓」高亮卡
  （品红图标 + 青粉渐变徽章 + 辉光），信号链路带保留在下方
- **CTA 气泡**：上移并居中浮在 cutout 正上方（`top:-2.2rem` + `z-index:2`），不再被角色遮挡

## 第四轮修订（用户反馈）

- **故障线随机化**：改为 JS 调度——随机间隔（3.2–8.4s）、随机方向
  （`is-down` 上→下 / `is-up` 下→上）、0.9s 快速扫过整个 Hero，单条或双条随机
- **Why 卡片结构修正**：cutout 回到**卡片内部**——每张卡 `grid 1fr auto`
  左文字右立绘，两个 cutout **等高一致**（`height: clamp(150px,20vw,230px)`，
  `object-fit: contain` + 底部对齐）；before 整体降饱和、after 青粉辉光

## 已知限制

- 切角 `clip-path` 在极窄视口下注意与 grid 间隙的贴合
- 中文字体经 Google Fonts 加载，网络受限时回退系统字体栈
- glitch / 进场 / 扫描动画依赖 JS + CSS animation，均已在 reduced-motion 下收敛为静态
