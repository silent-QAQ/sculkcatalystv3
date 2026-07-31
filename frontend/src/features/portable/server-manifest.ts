// SPDX-License-Identifier: Apache-2.0

export const SERVER_TEMPLATE_FORMAT = 'sculk-catalyst/server-template'
export const SERVER_TEMPLATE_MANIFEST_VERSION = 1
export const MAX_SERVER_TEMPLATE_FILE_BYTES = 64 * 1024

export interface ServerTemplate {
  id: string
  title: string
  description: string
  server: {
    name: string
    core: string
    minecraft_version: string
    memory_gb: number
    port: number
  }
  created_at: string
  updated_at: string
}

export interface ServerTemplateManifest {
  format: typeof SERVER_TEMPLATE_FORMAT
  manifest_version: typeof SERVER_TEMPLATE_MANIFEST_VERSION
  template: {
    title: string
    description: string
    server: ServerTemplate['server']
  }
}

const ROOT_KEYS = new Set(['format', 'manifest_version', 'template'])
const TEMPLATE_KEYS = new Set(['title', 'description', 'server'])
const SERVER_KEYS = new Set(['name', 'core', 'minecraft_version', 'memory_gb', 'port'])

function objectValue(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(`${label}必须是对象`)
  return value as Record<string, unknown>
}

function rejectUnknownKeys(value: Record<string, unknown>, allowed: Set<string>, label: string) {
  const unknown = Object.keys(value).filter(key => !allowed.has(key))
  if (unknown.length) throw new Error(`${label}包含不支持的字段：${unknown.join('、')}`)
}

function textValue(value: unknown, label: string, min: number, max: number) {
  if (typeof value !== 'string') throw new Error(`${label}必须是文本`)
  const text = value.trim()
  if (text.length < min || text.length > max) throw new Error(`${label}长度必须在 ${min}–${max} 个字符之间`)
  if ([...text].some(character => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127)) {
    throw new Error(`${label}不能包含控制字符`)
  }
  return text
}

function optionalTextValue(value: unknown, label: string, max: number) {
  if (typeof value !== 'string') throw new Error(`${label}必须是文本`)
  const text = value.trim()
  if (text.length > max) throw new Error(`${label}不能超过 ${max} 个字符`)
  if ([...text].some(character => character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127)) {
    throw new Error(`${label}不能包含控制字符`)
  }
  return text
}

function catalogValue(value: unknown, label: string, max: number) {
  const text = textValue(value, label, 1, max)
  if (!/^[\p{L}\p{N} ._+\-]+$/u.test(text)) throw new Error(`${label}包含不支持的字符`)
  return text
}

function integerValue(value: unknown, label: string, min: number, max: number) {
  if (typeof value !== 'number' || !Number.isInteger(value) || value < min || value > max) {
    throw new Error(`${label}必须是 ${min}–${max} 之间的整数`)
  }
  return value
}

export function createServerTemplate(value: unknown, id: string = crypto.randomUUID()): ServerTemplate {
  const template = objectValue(value, '模板')
  rejectUnknownKeys(template, TEMPLATE_KEYS, '模板')
  const server = objectValue(template.server, '服务器参数')
  rejectUnknownKeys(server, SERVER_KEYS, '服务器参数')
  const now = new Date().toISOString()
  return {
    id,
    title: textValue(template.title, '模板名称', 1, 64),
    description: optionalTextValue(template.description, '模板说明', 500),
    server: {
      name: textValue(server.name, '服务器名称', 1, 64),
      core: catalogValue(server.core, '服务端核心', 64),
      minecraft_version: catalogValue(server.minecraft_version, 'Minecraft 版本', 32),
      memory_gb: integerValue(server.memory_gb, '最大内存', 2, 64),
      port: integerValue(server.port, '服务器端口', 1024, 65535),
    },
    created_at: now,
    updated_at: now,
  }
}

export function normalizeStoredServerTemplate(value: unknown): ServerTemplate | null {
  try {
    const stored = objectValue(value, '模板')
    const template = createServerTemplate({
      title: stored.title,
      description: stored.description || '',
      server: stored.server,
    }, textValue(stored.id, '模板 ID', 1, 80))
    const createdAt = typeof stored.created_at === 'string' && !Number.isNaN(Date.parse(stored.created_at))
      ? stored.created_at
      : template.created_at
    const updatedAt = typeof stored.updated_at === 'string' && !Number.isNaN(Date.parse(stored.updated_at))
      ? stored.updated_at
      : createdAt
    return { ...template, created_at: createdAt, updated_at: updatedAt }
  } catch {
    return null
  }
}

export function parseServerTemplateManifest(raw: string): ServerTemplate {
  if (new Blob([raw]).size > MAX_SERVER_TEMPLATE_FILE_BYTES) throw new Error('配置文件不能超过 64 KiB')
  let parsed: unknown
  try { parsed = JSON.parse(raw) } catch { throw new Error('配置文件不是有效的 JSON') }
  const root = objectValue(parsed, '配置文件')
  rejectUnknownKeys(root, ROOT_KEYS, '配置文件')
  if (root.format !== SERVER_TEMPLATE_FORMAT) throw new Error(`配置格式必须是 ${SERVER_TEMPLATE_FORMAT}`)
  if (root.manifest_version !== SERVER_TEMPLATE_MANIFEST_VERSION) throw new Error('暂不支持该配置版本')
  return createServerTemplate(root.template)
}

export function exportServerTemplateManifest(template: ServerTemplate): string {
  const manifest: ServerTemplateManifest = {
    format: SERVER_TEMPLATE_FORMAT,
    manifest_version: SERVER_TEMPLATE_MANIFEST_VERSION,
    template: {
      title: template.title,
      description: template.description,
      server: { ...template.server },
    },
  }
  return `${JSON.stringify(manifest, null, 2)}\n`
}
