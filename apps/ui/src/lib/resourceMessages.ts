import { i18n } from './i18n.svelte';
export const resourceMessages = {
  vault: ['Keychain', '密钥链'], knownHosts: ['Known hosts', '已知主机'], logs: ['Diagnostics', '日志'],
  trustHint: ['Application trust store, not an OpenSSH known_hosts editor. Verify fingerprints out of band before trusting keys.', '应用信任库，并非 OpenSSH 文件编辑器。信任密钥前请通过独立渠道核对指纹。'],
  logsHint: ['Local, sanitized diagnostics only. Not terminal recordings or unlimited backend logs.', '仅显示本地脱敏诊断，并非终端录屏或无限后端日志。'],
  refresh: ['Refresh', '刷新'], add: ['Add host key', '新增主机密钥'], replace: ['Replace key', '替换密钥'], remove: ['Delete', '删除'], cancel: ['Cancel', '取消'], save: ['Save', '保存'],
  host: ['Host:port', '主机:端口'], key: ['Public key (type base64)', '公钥（类型 base64）'], fingerprint: ['SHA256 fingerprint', 'SHA256 指纹'],
  confirmTrust: ['I verified this fingerprint independently and trust this key. Replacement may hide a man-in-the-middle attack.', '我已通过独立渠道核对指纹并信任此密钥。替换可能掩盖中间人攻击。'],
  confirmDelete: ['Delete the pin for this host? The next connection will use first-use trust again.', '删除此主机的固定密钥？下次连接将重新使用首次信任。'],
  empty: ['No entries', '暂无条目'], category: ['Category', '分类'], level: ['Level', '级别'], all: ['All', '全部'], details: ['Details', '详情'], clear: ['Clear diagnostics', '清空日志'], export: ['Export sanitized JSON', '导出脱敏 JSON'], confirmClear: ['Clear all local diagnostic events?', '清空所有本地诊断事件？'], invalidKey: ['Enter a host:port and a valid SSH public key.', '请输入主机:端口和有效的 SSH 公钥。'],
  setup: ['Set up vault', '设置密钥链'], unavailable: ['Vault store unavailable. Restart the app to retry.', '密钥链存储不可用，请重启应用重试。'], encryptionHint: ['A strong master password encrypts this vault. Lost passwords cannot be recovered.', '请设置强主密码以加密密钥链。密码丢失后无法恢复。'], master: ['Master password', '主密码'], confirm: ['Confirm', '确认'], initialize: ['Initialize vault', '初始化密钥链'], unlock: ['Unlock', '解锁'], lock: ['Lock', '锁定'], newEntry: ['New entry', '新增条目'], changePassword: ['Change password', '修改密码'], currentPassword: ['Current password', '当前密码'], newPassword: ['New password', '新密码'], apply: ['Apply', '应用'], editEntry: ['Edit entry', '编辑条目'], label: ['Label', '名称'], kind: ['Kind', '类型'], username: ['Username (optional)', '用户名（可选）'], secret: ['Secret', '机密内容'], password: ['Password', '密码'], token: ['Token', '令牌'], privateKey: ['Private key', '私钥'], note: ['Encrypted note', '加密笔记'], entries: ['Entries', '条目'], reveal: ['Reveal', '显示'], copy: ['Copy', '复制'], edit: ['Edit', '编辑'], requiredPassword: ['Password required', '请输入密码'], mismatch: ['Passwords do not match', '两次密码不一致'], requiredLabel: ['Label required', '请输入名称'], bothPasswords: ['Both passwords required', '请输入当前密码和新密码'],
} satisfies Record<string, [string, string]>;
export function resourceText(key: keyof typeof resourceMessages): string {
  return resourceMessages[key][i18n.resolvedLocale === 'zh-CN' ? 1 : 0];
}
