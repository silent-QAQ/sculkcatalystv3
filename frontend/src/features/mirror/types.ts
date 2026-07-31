export type CatalogKind = 'core' | 'plugin' | 'skin' | 'bbmodel' | 'ui_texture' | 'skill' | 'plugin_config'
export type PluginCategory = 'mainstream' | 'open_source' | 'standard' | 'paid'
export type VersionStatus = 'draft' | 'published' | 'yanked'

export interface CatalogProject {
  kind: CatalogKind
  slug: string
  name: string
  summary: string
  description: string
  author: string
  homepage: string
  repository: string
  preview_url: string
  license: string
  plugin_category: PluginCategory | ''
  target_plugin: string
  tags: string[]
  color: string
  featured: boolean
  version_count: number
  published_versions: number
  latest_version?: string | null
  downloads: number
  minecraft_versions: string[]
  channels: string[]
  loaders: string[]
}

export interface CatalogVersion {
  id: string
  project: string
  version: string
  channel: string
  minecraft_versions: string[]
  loaders: string[]
  formats: string[]
  java_version?: number | null
  filename: string
  size: number
  sha256: string
  download_url: string
  content: string
  release_notes: string
  released_at: string
  status: VersionStatus
  downloads: number
}

export interface ResourceUpload {
  download_url: string
  object_path: string
  filename: string
  size: number
  sha256: string
}

export interface CatalogSummary {
  core_projects: number
  plugin_projects: number
  skin_projects: number
  bbmodel_projects: number
  ui_texture_projects: number
  skill_projects: number
  plugin_config_projects: number
  versions: number
  downloads: number
  published_versions: number
}

export interface ResolveResponse {
  kind: CatalogKind
  project: CatalogProject
  version: CatalogVersion
  download_path: string
}
