/** 首启引导内部类型与常量（契约类型见 `@/shared/contract/types`） */

import type { OnboardingDraft } from "@/shared/contract/types";
import { t } from "../../../i18n";

/** 引导步骤数（契约 §9：0–5） */
export const STEP_COUNT = 6;

export interface StepMeta {
  title: string;
  description: string;
  /** 是否必填（用于步骤条提示） */
  required: boolean;
}

export const STEP_META: StepMeta[] = [
  { title: t("欢迎与语言"), description: t("了解产品与热键，选择界面语言"), required: false },
  { title: t("AI 服务"), description: t("填写供应商与 Key，测试连接"), required: false },
  { title: t("翻译方式"), description: t("云端复用或本地离线模型"), required: false },
  { title: t("AI 模式"), description: t("为每个动作选择快速 / 深度"), required: false },
  { title: t("外观与行为"), description: t("主题、结果窗、开机自启、热键"), required: false },
  { title: t("隐私与完成"), description: t("上传开关、隐私同意与配置摘要"), required: true },
];

/** 归一化后的错误（与设置页同构，避免跨窗口依赖） */
export interface UiError {
  code: string;
  message: string;
  detail: string | null;
  retryable: boolean;
  fallback: string[];
}

/** 每个步骤组件的统一入参 */
export interface StepProps {
  draft: OnboardingDraft;
  /** 合并顶层字段（步骤内部自行构造嵌套对象） */
  onEdit: (patch: Partial<OnboardingDraft>) => void;
  /** 上报错误到外壳顶部提示 */
  onError: (error: unknown) => void;
}
