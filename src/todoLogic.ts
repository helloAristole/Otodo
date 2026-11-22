import type { Todo } from "./types";

function pad(number: number): string {
  return String(number).padStart(2, "0");
}

function localDayKey(date: Date): string {
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
}

function daysInMonth(dayKey: string): number {
  const [year, month] = dayKey.split("-").map(Number);
  return new Date(year, month, 0).getDate();
}

function isLeapYear(year: number): boolean {
  return (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
}

export function todoDayKey(todo: Todo): string {
  if (todo.due_at) return localDayKey(new Date(todo.due_at));
  if (todo.due_date) return todo.due_date;
  if (todo.created_at) return localDayKey(new Date(todo.created_at));
  return localDayKey(new Date());
}

export function todoDoneOnDay(todo: Todo, dayKey: string): boolean {
  if (todo.repeat && todo.repeat !== "none") {
    return todo.done_dates.split(" ").filter(Boolean).includes(dayKey);
  }
  return todo.done;
}

export function matchesTodoOnDay(todo: Todo, dayKey: string): boolean {
  if (todo.deleted) return false;
  if (todo.skip_dates?.split(" ").filter(Boolean).includes(dayKey)) return false;

  if (!todo.repeat || todo.repeat === "none") {
    if (todo.repeat_until && dayKey > todo.repeat_until) return false;
    return todoDayKey(todo) === dayKey;
  }

  const base = todo.due_date
    ? new Date(`${todo.due_date}T00:00:00`)
    : todo.due_at
      ? new Date(todo.due_at)
      : new Date();
  if (dayKey < localDayKey(base)) return false;
  if (todo.repeat_until && dayKey > todo.repeat_until) return false;

  const day = new Date(`${dayKey}T00:00:00`);
  switch (todo.repeat) {
    case "daily":
      return true;
    case "weekly":
      return day.getDay() === base.getDay();
    case "monthly":
      return day.getDate() === Math.min(base.getDate(), daysInMonth(dayKey));
    case "yearly": {
      const targetDay =
        base.getMonth() === 1 && base.getDate() === 29 && !isLeapYear(day.getFullYear())
          ? 28
          : base.getDate();
      return day.getMonth() === base.getMonth() && day.getDate() === targetDay;
    }
    default:
      return todoDayKey(todo) === dayKey;
  }
}

export function appendSkipDate(skipDates: string | null, dayKey: string): string {
  const dates = skipDates?.split(" ").filter(Boolean) ?? [];
  if (!dates.includes(dayKey)) dates.push(dayKey);
  return dates.join(" ");
}

export function countRemainingTodosForDay(todos: Todo[], dayKey: string): number {
  return todos.filter(
    (todo) => matchesTodoOnDay(todo, dayKey) && !todoDoneOnDay(todo, dayKey),
  ).length;
}
