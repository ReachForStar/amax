// 统计数据导出（CSV / JSON）：纯函数生成 + 系统 Picker 另存
// CSV 三节结构（[区间汇总] / [每日趋势] / [模型分布]）、字段口径与 BOM/CRLF 约定
// 与桌面版 dist/app.js buildCsv/buildExportData 逐字对齐
// （spec：docs/superpowers/specs/2026-08-02-stats-analytics-design.md 数据导出节）。
// 保存链路经 devecocli docs 查证（开发指南 save-user-file；API 参考 js-apis-file-picker）：
// - DocumentSaveOptions.newFileNames: Array<string> 预填文件名（可选字段）；
// - DocumentViewPicker(context).save(options): Promise<Array<string>>，取消返回空数组；
// - Picker 默认预置空文件（autoCreateEmptyFile 默认 true），应用经 fileIo 打开 URI 写入；
// - DocumentSaveOptions 无 MIME 字段，文件类型由扩展名决定，mimeType 仅作日志标识。
import { picker, fileIo } from '@kit.CoreFileKit';
import { common } from '@kit.AbilityKit';

// 导出区间（snake_case 为导出格式约定，不随内部驼峰状态改变）
export interface ExportRange {
  start_date: string;
  end_date: string;
}

// 区间汇总：官方模式 request_count 为区间总请求数；本地估算无法派生，置 null（CSV 跳过该行）
export interface ExportSummary {
  total_yuan: number;
  avg_yuan: number;
  peak_yuan: number;
  peak_date: string;
  total_tokens: number;
  request_count: number | null;
  days_with_usage: number;
}

// 每日明细：官方模式 input/output_tokens 有值，降级为 null；
// new_requests / remaining 来自本地快照（按日期合并），缺值为 null
export interface ExportDaily {
  date: string;
  yuan: number;
  tokens: number;
  input_tokens: number | null;
  output_tokens: number | null;
  new_requests: number | null;
  remaining: number | null;
}

// 模型分布项（percent_basis 为顶层字段，CSV 逐行附加）
export interface ExportModel {
  model: string;
  yuan: number;
  tokens: number;
  request_count: number;
  percent: number;
}

// 统一导出结构：与桌面版 buildExportData 一致，CSV / JSON 共用
export interface ExportData {
  app: string;
  exported_at: string;
  range: ExportRange;
  source: string;              // 'official' | 'local_estimate'
  percent_basis: string | null; // 降级模式无模型占比口径，置 null
  summary: ExportSummary | null;
  daily: ExportDaily[];
  models: ExportModel[];
}

// 用户在 Picker 对话框取消保存（save 返回空 URI 数组）：与写入失败区分，UI 提示中性文案
export class ExportCancelledError extends Error {
  constructor() {
    super('用户取消保存');
  }
}

// CSV 字段转义（同桌面版 csvEscape）：null → 空串；
// 含逗号 / 双引号 / 换行（\n 或 \r）时整体双引号包裹，内部引号加倍
export function csvEscape(value: string | number | null): string {
  if (value === null) {
    return '';
  }
  const text = String(value);
  if (/[",\n\r]/.test(text)) {
    return '"' + text.replace(/"/g, '""') + '"';
  }
  return text;
}

// 行数组 → 逗号拼接的 CSV 行
function csvRow(values: Array<string | number | null>): string {
  return values.map((v: string | number | null): string => csvEscape(v)).join(',');
}

// 生成三节 CSV：BOM（U+FEFF）前缀防 Excel 中文乱码，CRLF 行尾，节间空行分隔；
// 空节仅保留表头，文件末尾不带多余换行（均与桌面版一致）
export function buildStatsCsv(data: ExportData): string {
  const lines: string[] = [];
  lines.push('[区间汇总]');
  lines.push('指标,值');
  const s = data.summary;
  if (s !== null) {
    lines.push(`累计费用(元),${csvEscape(s.total_yuan)}`);
    lines.push(`日均费用(元),${csvEscape(s.avg_yuan)}`);
    lines.push(`单日峰值(元),${csvEscape(s.peak_yuan)}`);
    lines.push(`峰值日期,${csvEscape(s.peak_date)}`);
    lines.push(`累计Tokens,${csvEscape(s.total_tokens)}`);
    // 本地估算模式 request_count 为 null：不输出该行（同桌面版 != null 判断）
    if (s.request_count !== null) {
      lines.push(`区间总请求数,${csvEscape(s.request_count)}`);
    }
  }
  lines.push(`数据来源,${csvEscape(data.source)}`);
  lines.push('');
  lines.push('[每日趋势]');
  lines.push('date,yuan,tokens,input_tokens,output_tokens,new_requests,remaining');
  for (const d of data.daily) {
    lines.push(csvRow([d.date, d.yuan, d.tokens, d.input_tokens, d.output_tokens, d.new_requests, d.remaining]));
  }
  lines.push('');
  lines.push('[模型分布]');
  lines.push('model,yuan,tokens,request_count,percent,percent_basis');
  for (const m of data.models) {
    lines.push(csvRow([m.model, m.yuan, m.tokens, m.request_count, m.percent, data.percent_basis]));
  }
  return String.fromCharCode(0xFEFF) + lines.join('\r\n');
}

// 生成两空格缩进 JSON（同桌面版 JSON.stringify(data, null, 2)）
export function buildStatsJson(data: ExportData): string {
  return JSON.stringify(data, null, 2);
}

// 经系统另存对话框保存文本文件：Picker 返回用户选定的 URI，打开后写入 content。
// mimeType 无对应 Picker 参数（类型由扩展名决定），保留以对齐桌面导出契约并用于日志。
// 用户取消抛 ExportCancelledError，其余失败抛原始错误，由调用方提示，不静默吞错。
export async function saveTextFile(context: common.UIAbilityContext, fileName: string,
  content: string, mimeType: string): Promise<void> {
  const options = new picker.DocumentSaveOptions();
  options.newFileNames = [fileName];
  const documentPicker = new picker.DocumentViewPicker(context);
  const uris = await documentPicker.save(options);
  if (uris.length === 0) {
    throw new ExportCancelledError();
  }
  const uri = uris[0];
  // Picker 已预置空文件并授予临时读写权限；各步骤异常显式包装，便于调用方提示原因。
  // TRUNC：覆盖已有文件时先截断，避免旧尾部字节残留导致导出文件静默损坏（对齐桌面全量覆盖语义）
  let file: fileIo.File;
  try {
    file = fileIo.openSync(uri, fileIo.OpenMode.READ_WRITE | fileIo.OpenMode.TRUNC);
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    throw new Error(`打开保存文件失败：${reason}`);
  }
  try {
    fileIo.writeSync(file.fd, content);
    console.info(`导出成功（${mimeType}）：${uri}`);
  } catch (error) {
    const reason = error instanceof Error ? error.message : JSON.stringify(error);
    throw new Error(`写入导出文件失败：${reason}`);
  } finally {
    // 关闭失败不影响导出结果，仅记录原因（同 Store.ets ResultSet 关闭模式）
    try {
      fileIo.closeSync(file);
    } catch (closeError) {
      const reason = closeError instanceof Error ? closeError.message : JSON.stringify(closeError);
      console.error(`关闭导出文件失败：${reason}`);
    }
  }
}
