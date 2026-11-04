/** 引导第 0 步：欢迎与语言 —— 产品介绍、Ctrl+Q 说明、能力卡片、语言即时预览、隐私摘要 */

import type { StepProps } from "../types";
import { LANGUAGES } from "../presets";

const CAPABILITIES = [
  { title: "搜索", description: "框选文字或图片，立刻得到结论与来源" },
  { title: "翻译", description: "任意界面文字，一键翻译成你的语言" },
  { title: "解答", description: "不懂的报错、图表、术语，框选即解释" },
];

export function WelcomeStep(props: StepProps) {
  const { draft, onEdit } = props;
  const current = LANGUAGES.find((item) => item.value === draft.language) ?? LANGUAGES[0];

  return (
    <div className="ob-step">
      <div className="ob-hero">
        <h2 className="ob-hero-title">框一下，就懂了</h2>
        <p className="ob-hero-text">
          框选 AI 是一个常驻托盘的小工具：按下 <kbd className="ob-kbd">Ctrl+Q</kbd>{" "}
          框选屏幕任意区域，即可搜索、翻译或解答。
        </p>
      </div>

      <div className="ob-notice">
        Ctrl+Q 会注册为系统级全局快捷键，可能占用其他软件的相同组合键；如冲突可在后续步骤或设置中修改。
      </div>

      <div className="ob-capabilities">
        {CAPABILITIES.map((capability) => (
          <div key={capability.title} className="ob-capability">
            <span className="ob-capability-title">{capability.title}</span>
            <span className="ob-capability-desc">{capability.description}</span>
          </div>
        ))}
      </div>

      <div className="ob-block">
        <h3 className="ob-block-title">界面语言</h3>
        <p className="ob-hint">切换后仅预览文案；保存后立即生效，不影响引导流程。</p>
        <div className="ob-langs">
          {LANGUAGES.map((language) => (
            <button
              key={language.value}
              type="button"
              className={`ob-lang${draft.language === language.value ? " is-selected" : ""}`}
              onClick={() => onEdit({ language: language.value })}
            >
              <span className="ob-lang-label">{language.label}</span>
              <span className="ob-lang-preview">{language.preview}</span>
            </button>
          ))}
        </div>
        <p className="ob-hint">当前预览：{current ? current.preview : ""}</p>
      </div>

      <p className="ob-privacy">
        隐私摘要：默认不上传任何截图；只有你主动触发云端动作时，才会把对应文本或图片发送给你自己配置的模型服务。
        不采集遥测，配置与历史默认只存本机。
      </p>
    </div>
  );
}
