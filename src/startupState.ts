export type StartupScreen = "loading" | "recovery" | "ready";

/**
 * 启动状态按安全优先级决策：加载中永远不能进入业务界面，
 * 初始化错误只能进入恢复页。
 */
export function getStartupScreen(
  loading: boolean,
  initializationError: string | null
): StartupScreen {
  if (loading) return "loading";
  if (initializationError) return "recovery";
  return "ready";
}
