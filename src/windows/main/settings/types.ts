/**
 * 设置页内部类型：分组入参、草稿错误模型。
 *
 * 契约类型来自 `@/shared/contract/types`；这里只放页面内部约定，不新增契约。
 */

import type { Config, ConfigView, DeepPartial } from "@/shared/contract/types";

/** 设置页分组标识（与导航一一对应） */
export type SectionId =
  "general" | "appearance" | "ai" | "translate" | "actions" | "advanced" | "about";

/** 归一化后的错误（由 AppError 收敛而来，字符串兜底） */
export interface UiError {
  code: string;
  message: string;
  detail: string | null;
  retryable: boolean;
  fallback: string[];
}

/** 一次 `config_patch` 的入参形状（与契约 DeepPartial<Config> 对应） */
export type ConfigPatch = { [K in keyof Config]?: DeepPartial<Config[K]> };

/** 每个分组组件的统一入参：草稿就地编辑，应用时只提交本组差异 */
export interface SectionProps {
  /** 正在编辑的完整草稿 */
  draft: ConfigView;
  /** 上次从后端同步的配置（用于计算差异） */
  baseline: ConfigView;
  /** 是否有分组保存请求正在进行 */
  saving: boolean;
  /** 就地修改草稿（不可变更新） */
  onEdit: (updater: (draft: ConfigView) => ConfigView) => void;
  /** 提交本组差异；返回是否成功 */
  onApply: (patch: ConfigPatch, label: string) => Promise<boolean>;
  /** 用命令返回的完整配置替换草稿（导入 / 恢复默认 / 外部变更） */
  onReplace: (view: ConfigView) => void;
  /** 上报错误到设置页顶部提示 */
  onError: (error: unknown) => void;
}
