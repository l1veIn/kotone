// 滚动进场 + 导航 scroll-spy（reduced-motion 下全部瞬时呈现）
const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

const revealEls = Array.from(document.querySelectorAll("[data-reveal]"));
if (reduce || !("IntersectionObserver" in window)) {
  revealEls.forEach((el) => el.classList.add("is-in"));
} else {
  const io = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.isIntersecting) {
          e.target.classList.add("is-in");
          io.unobserve(e.target);
        }
      }
    },
    { threshold: 0.12, rootMargin: "0px 0px -8% 0px" }
  );
  revealEls.forEach((el) => io.observe(el));
}

// scroll-spy：高亮当前可视区块的导航项
const spyIds = ["features", "workflow", "proof", "privacy"];
const spyLinks = Array.from(
  document.querySelectorAll<HTMLElement>(".nav__link[href^='#']")
);
const spyMap = new Map(spyIds.map((id) => [
  id,
  { section: document.getElementById(id), link: spyLinks.find((a) => a.getAttribute("href") === `#${id}`) },
]).filter(([, v]) => v.section));

if ("IntersectionObserver" in window && spyMap.size) {
  const spy = new IntersectionObserver(
    (entries) => {
      for (const e of entries) {
        if (e.isIntersecting) {
          spyMap.forEach(({ link }, id) => link?.classList.toggle("is-active", id === e.target.id));
        }
      }
    },
    { rootMargin: "-45% 0px -50% 0px" }
  );
  spyMap.forEach(({ section }) => spy.observe(section!));
}
