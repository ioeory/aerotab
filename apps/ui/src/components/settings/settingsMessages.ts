import { i18n } from '../../lib/i18n.svelte';

/** Integration fallback: central i18n keys take precedence when supplied. */
export const settingsMessages: Record<string, [string, string]> = {
  'settings.search': ['Search settings', '搜索设置'],
  'settings.searchHint': ['Search section titles and descriptions', '搜索分区标题与说明'],
  'settings.saved': ['All changes saved', '所有更改已保存'],
  'settings.saveAll': ['Save all changes', '保存全部更改'],
  'settings.draftHint': ['Changes stay in your draft when switching sections.', '切换分区不会丢失尚未保存的更改。'],
  'settings.appearance.mode': ['Color mode', '颜色模式'],
  'settings.appearance.light': ['Light', '浅色'],
  'settings.appearance.system': ['System', '跟随系统'],
  'settings.appearance.dark': ['Dark', '深色'],
  'settings.appearance.palette': ['Interface palette', '界面调色板'],
  'settings.appearance.independent': ['Interface palettes do not change terminal colors or profile overrides.', '界面调色板不会改变终端颜色或连接配置中的显式覆盖。'],
  'settings.appearance.accent': ['Custom accent color', '自定义强调色'],
  'settings.appearance.accentHint': ['Optional override for highlights and focus. Status colors stay unchanged.', '可选覆盖高亮与焦点颜色，不改变成功、警告或危险颜色。'],
  'settings.appearance.preview': ['Preview', '预览'],
  'settings.appearance.apply': ['Apply', '应用'],
  'settings.appearance.cancelPreview': ['Cancel preview', '取消预览'],
  'settings.appearance.reset': ['Restore appearance defaults', '恢复外观默认值'],
  'settings.appearance.previewHint': ['Preview affects only this window. Apply or Save all changes to keep it.', '预览仅影响当前窗口；应用或保存全部更改后才会持久化。'],
  'settings.appearance.font': ['Terminal font', '终端字体'],
  'settings.appearance.family': ['Font family', '字体'],
  'settings.appearance.fallback': ['Fallback font', '后备字体'],
  'settings.appearance.fallbackHint': ['Appended to the family list when glyphs are missing.', '当字体缺少字符时，使用列表末尾追加的后备字体。'],
  'settings.appearance.size': ['Size (px)', '字号（px）'],
  'settings.appearance.weight': ['Normal weight', '常规字重'],
  'settings.appearance.boldWeight': ['Bold weight', '粗体字重'],
  'settings.appearance.ligatures': ['Enable font ligatures', '启用字体连字'],
  'settings.appearance.cursor': ['Cursor', '光标'],
  'settings.appearance.cursorShape': ['Cursor shape', '光标形状'],
  'settings.appearance.block': ['Block', '方块'],
  'settings.appearance.bar': ['Bar', '竖线'],
  'settings.appearance.underline': ['Underline', '下划线'],
  'settings.appearance.layout': ['Terminal spacing and contrast', '终端间距与对比度'],
  'settings.appearance.linePadding': ['Line padding (px)', '行内边距（px）'],
  'settings.appearance.linePaddingHint': ['Adds vertical spacing between lines.', '在终端行之间增加垂直间距。'],
  'settings.appearance.contrast': ['Minimum contrast ratio', '最小对比度'],
  'settings.appearance.contrastHint': ['1 = off, 4.5 = WCAG AA, 7 = WCAG AAA.', '1 = 关闭，4.5 = WCAG AA，7 = WCAG AAA。'],
  'settings.appearance.customCss': ['Custom CSS', '自定义 CSS'],
  'settings.appearance.cssLabel': ['CSS injected into the application document', '注入应用页面的 CSS'],
  'settings.appearance.cssHint': ['Use with care: invalid CSS is ignored, but selectors can hide the interface.', '请谨慎使用：无效 CSS 会被忽略，但选择器可能隐藏界面。'],
};

export function settingsText(key: string): string {
  const value = i18n.t(key);
  return value !== key ? value : settingsMessages[key]?.[i18n.resolvedLocale === 'zh-CN' ? 1 : 0] ?? key;
}
