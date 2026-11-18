// 当前角色的名字（设置 agent.identity.name；没设时是默认角色琴音）。界面上提到“她”的地方都用它，
// 不写死某个角色的名字。
import { derived, type Readable } from "svelte/store";
import { settingsStore } from "./ui";

export const DEFAULT_COMPANION_NAME = "琴音";

export const companionName: Readable<string> = derived(settingsStore, ($settings) =>
  $settings?.agent?.identity?.name?.trim() || DEFAULT_COMPANION_NAME);
