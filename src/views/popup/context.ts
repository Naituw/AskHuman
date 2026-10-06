// 弹窗共享上下文：PopupView 在 setup 里调用 createPopupContext()（组装 usePopupCore 并
// provide），各区块子组件用 usePopupContext() 注入取用。类型由返回值推导。
import { inject, provide, type InjectionKey } from "vue";
import { usePopupCore, type PopupScope } from "./usePopupCore";

export function createPopupContext(scope?: PopupScope) {
  const ctx = usePopupCore(scope);
  provide(PopupCtxKey, ctx);
  return ctx;
}

export type PopupContext = ReturnType<typeof createPopupContext>;

export const PopupCtxKey: InjectionKey<PopupContext> = Symbol("popup-ctx");

export function usePopupContext(): PopupContext {
  const ctx = inject(PopupCtxKey);
  if (!ctx) throw new Error("popup context not provided");
  return ctx;
}
