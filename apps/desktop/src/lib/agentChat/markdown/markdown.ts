// 安全 Markdown：只用 marked 的词法分析得到语法树，再由 Svelte 组件逐个节点渲染成普通元素。
// 不使用 innerHTML：模型写的 HTML 原样显示成文字，图片不自动加载（只显示说明文字），
// 链接只接受 http(s)，点了用系统浏览器打开。Kotone 自己的实现。
import { Lexer, type Token, type Tokens } from "marked";

export type { Token, Tokens };

export function lexMarkdown(text: string): Token[] {
  try {
    return Lexer.lex(text, { gfm: true, breaks: true });
  } catch {
    // 解析失败就当普通文字显示
    return [{ type: "paragraph", raw: text, text, tokens: [{ type: "text", raw: text, text }] } as Tokens.Paragraph];
  }
}

export function isSafeHttpUrl(href: string): boolean {
  try {
    const url = new URL(href);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

/** 节点里没有子节点时的纯文字（未知节点的兜底）。 */
export function tokenText(token: Token): string {
  const t = token as { text?: unknown; raw?: unknown };
  return typeof t.text === "string" ? t.text : typeof t.raw === "string" ? t.raw : "";
}
