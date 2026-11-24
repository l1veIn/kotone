// 工具没给页面数据时（不在对局里、客户端没开、数据开关关着、查不到……）页面上的那句话：按工具结果里的
// `status` 和 `reason` 选翻译好的文案（docs/design/lol-app.md 第一节 10），不直接显示给琴音的原话（那句只有中文，
// 放在 title 里方便排查）。
import { m } from "./i18n.svelte";

export interface ToolState {
  status: string;
  /** `not_in_game` 的细分：`client_closed` / `not_in_game` / `champ_select` / `loading` / `ended`。 */
  reason?: string;
  /** 给琴音的原话。 */
  text: string;
}

export interface StateView {
  /** 页面上的那句话。 */
  text: string;
  /** 阶段牌上的短说法。 */
  pill: string;
  /** 阶段牌的点：空心（客户端没开）、灰（不在对局里）、蓝（在阶段里）。 */
  dot: "off" | "idle" | "";
  /** 要不要给“去设置”。 */
  settings: boolean;
}

export function stateView(st: ToolState): StateView {
  const reason = st.reason ?? "";
  switch (st.status) {
    case "disabled":
      return { text: m.plugin_lol_state_disabled(), pill: m.plugin_lol_phase_none(), dot: "off", settings: true };
    case "not_in_client":
      return { text: m.plugin_lol_state_client_closed(), pill: m.plugin_lol_phase_client_closed(), dot: "off", settings: false };
    case "not_in_game":
      if (reason === "client_closed")
        return { text: m.plugin_lol_state_client_closed(), pill: m.plugin_lol_phase_client_closed(), dot: "off", settings: false };
      if (reason === "champ_select")
        return { text: m.plugin_lol_state_champ_select(), pill: m.plugin_lol_phase_select(), dot: "", settings: false };
      if (reason === "loading")
        return { text: m.plugin_lol_state_loading(), pill: m.plugin_lol_phase_loading(), dot: "", settings: false };
      if (reason === "ended")
        return { text: m.plugin_lol_state_ended(), pill: m.plugin_lol_phase_post(), dot: "idle", settings: false };
      return { text: m.plugin_lol_not_in_game(), pill: m.plugin_lol_phase_none(), dot: "idle", settings: false };
    case "not_found":
      return { text: m.plugin_lol_state_not_found(), pill: "", dot: "idle", settings: false };
    case "ambiguous":
      return { text: m.plugin_lol_state_ambiguous(), pill: "", dot: "idle", settings: false };
    default:
      return { text: m.plugin_lol_state_error(), pill: "", dot: "idle", settings: false };
  }
}
