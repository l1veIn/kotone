// Kotone 真机测试用插件（scripts/manual-e2e/browser_probe_real.py）：确认浏览器插件真的装上、跑起来了。
(() => {
  const mark = () => {
    if (!document.title.startsWith('[KOTONE-EXT] ')) {
      document.title = '[KOTONE-EXT] ' + document.title;
    }
    document.documentElement.setAttribute('data-kotone-ext', '1');
  };
  mark();
  new MutationObserver(mark).observe(document.querySelector('title') || document.head || document.documentElement, {
    childList: true,
    subtree: true,
    characterData: true,
  });
})();
