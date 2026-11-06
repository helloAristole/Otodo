export type RepeatVal = "none" | "daily" | "weekly" | "monthly" | "yearly";

export interface Todo {
  id: string;
  title: string;
  note: string | null;
  due_at: string | null; // ISO 时间戳（UTC）
  all_day: boolean;
  due_date: string | null; // YYYY-MM-DD
  done: boolean;
  sort_order: number;
  deleted: boolean;
  repeat: RepeatVal; // 重复规则
  done_dates: string; // 重复待办按天完成的 ymd 列表，空格分隔
  repeat_until: string | null; // 重复结束日期 YYYY-MM-DD（本地专属）
  skip_dates: string | null; // 跳过的日期 ymd 列表，空格分隔（本地专属）
  created_at?: string | null;
  updated_at?: string | null;
}
