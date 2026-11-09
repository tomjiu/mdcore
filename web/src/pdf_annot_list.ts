// #475(长尾⑨缩小版):web PDF 注释列表面板——会话 AnnotListed 数据的
// 可视化清单 + 点击跳页。
//
// 模块定位(与 pdf_view 分工):本模块 = 列表行形状 + DOM(纯函数——
// kind 标签/预览/行 DOM);pdf_view.ts = 接线(this.annotations 会话真
// 源 + 增删后重渲染 + 跳页回调)。
//
// 诚实边界(登记):
//   - v0 行 = kind 徽标 + 页号 + FreeText 文本预览;Note 无文本载荷
//     (v0 会话 Note 无 /Contents 面——pdf_view noteText 为宿主显示层),
//     行内不显示;
//   - 筛选/排序/导出(Acrobat 注释管理完整面)不在本切片——长尾⑨原文
//     的完整面,另行轮次。

/** 会话注释(AnnotationJson 形状——pdf_view 已有类型,这里鸭子化避免环导)。 */
export interface AnnotRowLike {
  id: number;
  page: number;
  kind:
    | { Highlight: { quads: unknown[]; color: unknown } }
    | { Note: { rect: unknown } }
    | { FreeText: { rect: unknown; text: string } }
    | { Ink: { strokes: unknown; color: unknown; width: number } };
  author: string;
}

/** kind 名(serde 外标签——单一出口)。 */
export function annotKindName(kind: AnnotRowLike['kind']): string {
  if ('Highlight' in kind) return 'Highlight';
  if ('Note' in kind) return 'Note';
  if ('FreeText' in kind) return 'FreeText';
  return 'Ink';
}

/** 行预览文本(FreeText 有载荷文本;其余 v0 无文本面——空串)。 */
export function annotPreview(kind: AnnotRowLike['kind']): string {
  if ('FreeText' in kind) return kind.FreeText.text;
  return '';
}

/** 列表 DOM:行 = 徽标 + 页号 + 预览;点击行 = 跳页(onJump 携 0 基页)。 */
export function renderAnnotListDom(
  container: HTMLElement,
  annots: readonly AnnotRowLike[],
  onJump: (page: number, id: number) => void,
  labels: { kind: Record<string, string>; page: (n: number) => string },
): void {
  container.replaceChildren();
  if (!annots.length) return;
  const ul = document.createElement('ul');
  ul.className = 'pdf-annots-list';
  for (const a of annots) {
    const name = annotKindName(a.kind);
    const li = document.createElement('li');
    li.className = 'pdf-annots-item';
    li.dataset.id = String(a.id);
    li.dataset.page = String(a.page);
    li.dataset.kind = name;

    const badge = document.createElement('span');
    badge.className = `pdf-annots-badge pdf-annots-k-${name.toLowerCase()}`;
    badge.textContent = labels.kind[name] ?? name;

    const page = document.createElement('span');
    page.className = 'pdf-annots-page';
    page.textContent = labels.page(a.page + 1);

    const preview = document.createElement('span');
    preview.className = 'pdf-annots-preview';
    const pv = annotPreview(a.kind);
    preview.textContent = pv ? pv.slice(0, 80) : '';
    preview.title = pv;

    li.append(badge, page, preview);
    li.addEventListener('click', () => onJump(a.page, a.id));
    ul.appendChild(li);
  }
  container.appendChild(ul);
}
