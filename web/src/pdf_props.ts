// #472(长尾⑦):web PDF 文档属性面板——pdf.js getMetadata() 的 /Info
// 字段 + 会话 meta(文件名/页数)+ 字节数 + 打开扫描评估(加密/警告)
// 汇总展示。
//
// 模块定位(与 pdf_view 分工):本模块 = 纯数据形状 + 面板 DOM(字段
// 表——值缺失 = 「—」诚实空态);pdf_view.ts = 接线(getMetadata 取数
// + 会话/评估汇总 + 抽屉开合)。
//
// 诚实边界(登记):
//   - XMP 原始 XML(metadata 面)不做——长尾⑦ 的 XMP 半边,v0 只做
//     /Info 结构化字段(getMetadata().info);
//   - PDF 日期串逐字展示(D:YYYYMMDDHHmmSS±HH'mm' 原样)——不猜时区
//     语义(引擎面 /M 时区 UTC 近似的精度面登记同源);

/** 文档属性面板数据(/Info 字段 + 会话/文件面汇总——display 单一数据源)。 */
export interface DocProps {
  title: string | null;
  author: string | null;
  subject: string | null;
  keywords: string | null;
  producer: string | null;
  creator: string | null;
  creationDate: string | null;
  modDate: string | null;
  pdfVersion: string | null;
  fileName: string;
  pageCount: number;
  fileSize: number;
  encrypted: boolean;
  warnings: number;
}

/** pdf.js getMetadata().info 的最小结构面(鸭子——不依赖精确类型)。 */
export interface PdfInfoLike {
  Title?: string;
  Author?: string;
  Subject?: string;
  Keywords?: string;
  Producer?: string;
  Creator?: string;
  CreationDate?: string;
  ModDate?: string;
  PDFFormatVersion?: string;
}

/** 字段行描述(label key 由 i18n 注入——本模块零 i18n 依赖,可单测)。 */
export interface PropField {
  key: string;
  label: string;
  value: string;
}

/** DocProps → 展示字段列表(null → 空态占位——由调用方注入占位串)。 */
export function propFields(
  p: DocProps,
  emptyMark: string,
  labels: Record<string, string>,
  formatSize: (bytes: number) => string,
  formatEncrypted: (b: boolean) => string,
): PropField[] {
  const v = (s: string | null): string => (s && s.trim() ? s : emptyMark);
  return [
    { key: 'title', label: labels.title, value: v(p.title) },
    { key: 'author', label: labels.author, value: v(p.author) },
    { key: 'subject', label: labels.subject, value: v(p.subject) },
    { key: 'keywords', label: labels.keywords, value: v(p.keywords) },
    { key: 'producer', label: labels.producer, value: v(p.producer) },
    { key: 'creator', label: labels.creator, value: v(p.creator) },
    { key: 'created', label: labels.created, value: v(p.creationDate) },
    { key: 'modified', label: labels.modified, value: v(p.modDate) },
    { key: 'pdfVersion', label: labels.pdfVersion, value: v(p.pdfVersion) },
    { key: 'pages', label: labels.pages, value: String(p.pageCount) },
    { key: 'fileSize', label: labels.fileSize, value: formatSize(p.fileSize) },
    { key: 'encrypted', label: labels.encrypted, value: formatEncrypted(p.encrypted) },
    { key: 'warnings', label: labels.warnings, value: String(p.warnings) },
  ];
}

/** 字段表 DOM(dl 语义表——每行 data-field 锚,e2e 逐字段断言面)。 */
export function renderPropsDom(container: HTMLElement, fields: readonly PropField[]): void {
  container.replaceChildren();
  const dl = document.createElement('dl');
  dl.className = 'pdf-props-list';
  for (const f of fields) {
    const row = document.createElement('div');
    row.className = 'pdf-props-row';
    row.dataset.field = f.key;
    const dt = document.createElement('dt');
    dt.textContent = f.label;
    const dd = document.createElement('dd');
    dd.textContent = f.value;
    dd.title = f.value;
    row.append(dt, dd);
    dl.appendChild(row);
  }
  container.appendChild(dl);
}
