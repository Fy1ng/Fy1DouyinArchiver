<script lang="ts">
  import type { Snippet } from "svelte";

  const steps = [
    { title: "账号登录", description: "连接独立的抖音账号资料" },
    { title: "同步设置", description: "选择来源、范围与周期" },
    { title: "数据与桌面", description: "确认存储位置与启动方式" },
    { title: "确认设置", description: "复核并开始使用 Fy1DouyinArchiver" },
  ] as const;

  let {
    step,
    appIconUrl,
    children,
  }: {
    step: 0 | 1 | 2 | 3;
    appIconUrl: string;
    children?: Snippet;
  } = $props();
</script>

<div
  class="onboarding-workspace"
  role="dialog"
  aria-modal="true"
  aria-label="首次设置"
>
  <aside class="onboarding-brand-pane">
    <div class="onboarding-brand">
      <img src={appIconUrl} alt="Fy1DouyinArchiver" draggable="false" />
      <div>
        <strong>Fy1DouyinArchiver</strong>
        <span>抖音内容本地归档</span>
      </div>
    </div>

    <div class="onboarding-brand-copy">
      <p>你的记录只保存在本机，由你选择同步范围、存储位置和后台行为。</p>
    </div>

    <ol class="onboarding-step-list" aria-label="设置进度">
      {#each steps as item, index}
        <li
          class:active={index === step}
          class:complete={index < step}
          aria-current={index === step ? "step" : undefined}
        >
          <span>{index + 1}</span>
          <div>
            <strong>{item.title}</strong>
            <small>{item.description}</small>
          </div>
        </li>
      {/each}
    </ol>

    <small class="onboarding-local-note">非官方工具 · 数据默认留在本机</small>
  </aside>

  <div class="onboarding-form-pane">
    <header class="onboarding-form-header">
      <span>首次设置 {step + 1} / 4</span>
      <h2>{steps[step].title}</h2>
      <p>{steps[step].description}</p>
    </header>
    <div class="onboarding-form-content">
      {@render children?.()}
    </div>
  </div>
</div>
