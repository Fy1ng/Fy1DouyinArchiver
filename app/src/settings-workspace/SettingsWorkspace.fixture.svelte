<script lang="ts">
  import SettingsWorkspace from "./SettingsWorkspace.svelte";
</script>

<SettingsWorkspace saveState="saved">
  <section data-settings-category="general" data-settings-panel="appearance" data-settings-panel-label="外观" data-settings-search-target>
    <h3>外观</h3>
    <label>主题<select aria-label="主题"><option>深色</option></select></label>
  </section>
  <section data-settings-category="general" data-settings-panel="desktop" data-settings-panel-label="桌面行为" data-settings-search-target>
    <h3>桌面行为</h3>
    <label>关闭行为<input aria-label="关闭行为" /></label>
  </section>
  <section data-settings-category="notifications" data-settings-panel="notification-behavior" data-settings-panel-label="消息行为" data-settings-search-target>
    <h3>通知</h3>
    <label>消息提示停留时间<input aria-label="消息提示停留时间" /></label>
  </section>
</SettingsWorkspace>
