// shadcn-svelte 约定的工具函数（https://shadcn-svelte.com，MIT）：合并 Tailwind 类名，
// 以及组件 props 的辅助类型。基础组件（lib/components/ui）都从这里取。
import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export type WithElementRef<T, Element extends HTMLElement = HTMLElement> = T & {
  ref?: Element | null;
};

export type WithoutChild<T> = Omit<T, "child">;
export type WithoutChildren<T> = Omit<T, "children">;
export type WithoutChildrenOrChild<T> = Omit<T, "children" | "child">;
