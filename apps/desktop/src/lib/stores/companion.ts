// 当前角色的名字（设置 agent.identity.name；没设时是默认角色琴音）。界面上提到“她”的地方都用它，
// 不写死某个角色的名字。
import { derived, type Readable } from "svelte/store";
import { settingsStore } from "./ui";

// i18n-ignore-start：角色内容（名字、性格、唤醒词规则，和后端一致），不是界面文字
export const DEFAULT_COMPANION_NAME = "琴音";
// i18n-ignore-end

export const companionName: Readable<string> = derived(settingsStore, ($settings) =>
  $settings?.agent?.identity?.name?.trim() || DEFAULT_COMPANION_NAME);
