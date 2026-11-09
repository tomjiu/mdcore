// #463（pdf-acrobat-parity 长尾④）：web PDF 大纲树——pdf.js getOutline
// 读面 → 树 UI + 点击跳页。
//
// 模块定位（与 pdf_view 的分工）：
//   - 本模块 = 纯转换 + 渲染：pdf.js 大纲项（含命名目标解析结果）→
//     引擎 OutlineTree serde 形状（{"roots":[{title,target:{Page:N},
//     children}]}——0 基页号，与 crates/pdf/src/outline.rs 同一 serde
//     面）+ 可折叠树 DOM。不持有 pdf.js 依赖——目标解析回调由
//     pdf_view 注入（dest 数组 → 0 基页号），保持本模块可单测。
//   - pdf_view.ts = 接线：getOutline() 取数 / 解析回调 / 会话 ctor
//     注入（buildOutlineJson 恒空树 → 实数据）/ 跳页回调。
//
// 诚实边界（登记）：
//   - 目标不可解析的节点（悬空命名目标/远程目标）**自身跳过、子级上提**
//     ——不发明页号（引擎面 OutlineTarget 只有 Page 变体，注入越界值
//     会被 PdfSession::open 整档拒绝，得不偿失）；跳过数入 probe
//     （skipped），不静默。
//   - 树节点文本 = /Title 逐字（不截断不修饰——「标题文本逐字来自文档
//     大纲」是 #463 验收 1）。

/** 引擎 OutlineNode serde 形状（crates/pdf/src/outline.rs——单一形状，会话 ctor 与树 UI 同源）。 */
export interface OutlineNodeJson {
  title: string;
  target: { Page: number };
  children: OutlineNodeJson[];
}

/** pdf.js getOutline 项的最小结构面（本模块不 import pdfjs 类型——结构鸭子即可）。 */
export interface PdfOutlineItemLike {
  title: string | undefined;
  dest: unknown;
  items?: PdfOutlineItemLike[] | null;
}

/** 转换产物：树 + 跳过数（诚实登记面）。 */
export interface OutlineBuildResult {
  tree: OutlineNodeJson[];
  skipped: number;
}

/**
 * pdf.js 大纲项 → 引擎 OutlineTree 形状。
 *
 * @param resolvePage dest 解析回调（pdf.js dest 数组/命名目标 → 0 基页号；
 *                    不可解析返回 null）。约定异步——命名目标解析走
 *                    doc.getDestination + doc.getPageIndex。
 */
export async function buildOutlineTree(
  items: readonly PdfOutlineItemLike[] | null | undefined,
  resolvePage: (dest: unknown) => Promise<number | null>,
): Promise<OutlineBuildResult> {
  let skipped = 0;

  async function walk(
    list: readonly PdfOutlineItemLike[],
    out: OutlineNodeJson[],
  ): Promise<void> {
    for (const item of list) {
      const title = typeof item.title === 'string' ? item.title : '';
      const pageIdx = await resolvePage(item.dest);
      if (pageIdx === null || pageIdx < 0) {
        // 目标不可解析：自身跳过、子级上提（不丢后代导航面）。
        skipped += 1;
        if (item.items?.length) await walk(item.items, out);
        continue;
      }
      const node: OutlineNodeJson = {
        title,
        target: { Page: pageIdx },
        children: [],
      };
      if (item.items?.length) await walk(item.items, node.children);
      out.push(node);
    }
  }

  const tree: OutlineNodeJson[] = [];
  if (items?.length) await walk(items, tree);
  return { tree, skipped };
}

/** 引擎 OutlineTree serde 全形状（pdf_view 会话 ctor 注入——与树 UI 同一数据源）。 */
export function outlineTreeJson(roots: readonly OutlineNodeJson[]): string {
  // 空树走引擎 Default 形状（{"roots":[]}——与旧恒空面兼容）。
  return JSON.stringify({ roots });
}

/** 先序平铺（DOM data-title 与 probe count 同一序——文档序）。 */
export function flattenOutline(roots: readonly OutlineNodeJson[]): OutlineNodeJson[] {
  const out: OutlineNodeJson[] = [];
  const stack = [...roots].reverse();
  while (stack.length) {
    const n = stack.pop() as OutlineNodeJson;
    out.push(n);
    for (let i = n.children.length - 1; i >= 0; i--) stack.push(n.children[i]);
  }
  return out;
}

/**
 * 树 DOM：每节点一行（data-title + data-page），子级可折叠（分支节点
 * 点击图标区折叠/展开——**不触发跳页**；点击标题区才跳页，Acrobat 同款
 * 交互语义）。样式类名挂 index.html（#pdf-outline-drawer 系）。
 */
export function renderOutlineDom(
  container: HTMLElement,
  roots: readonly OutlineNodeJson[],
  onJump: (pageIdx: number) => void,
): void {
  container.replaceChildren();
  if (!roots.length) return;

  const buildList = (nodes: readonly OutlineNodeJson[], depth: number): HTMLElement => {
    const ul = document.createElement('ul');
    ul.className = 'pdf-outline-list';
    for (const n of nodes) {
      const li = document.createElement('li');
      li.className = 'pdf-outline-item';
      li.dataset.title = n.title;
      li.dataset.page = String(n.target.Page);

      const row = document.createElement('div');
      row.className = 'pdf-outline-row';
      row.style.paddingLeft = `${depth * 14 + 4}px`;

      const label = document.createElement('button');
      label.type = 'button';
      label.className = 'pdf-outline-label';
      label.textContent = n.title;
      label.title = n.title;
      label.addEventListener('click', () => onJump(n.target.Page));

      if (n.children.length) {
        const toggle = document.createElement('button');
        toggle.type = 'button';
        toggle.className = 'pdf-outline-toggle';
        toggle.setAttribute('aria-expanded', 'true');
        toggle.textContent = '▾';
        toggle.addEventListener('click', () => {
          const expanded = toggle.getAttribute('aria-expanded') === 'true';
          toggle.setAttribute('aria-expanded', expanded ? 'false' : 'true');
          toggle.textContent = expanded ? '▸' : '▾';
          childWrap.hidden = expanded;
        });
        row.append(toggle, label);
      } else {
        const spacer = document.createElement('span');
        spacer.className = 'pdf-outline-leaf';
        row.append(spacer, label);
      }
      li.appendChild(row);

      const childWrap = document.createElement('div');
      if (n.children.length) childWrap.appendChild(buildList(n.children, depth + 1));
      li.appendChild(childWrap);

      ul.appendChild(li);
    }
    return ul;
  };

  container.appendChild(buildList(roots, 0));
}
