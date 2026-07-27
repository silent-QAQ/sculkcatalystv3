export type CatalogKind = 'core' | 'plugin'
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
  java_version?: number | null
  filename: string
  size: number
  sha256: string
  download_url: string
  release_notes: string
  released_at: string
  status: VersionStatus
  downloads: number
}

export interface CatalogSummary {
  core_projects: number
  plugin_projects: number
  versions: number
  downloads: number
}

export interface ResolveResponse {
  kind: CatalogKind
  project: CatalogProject
  version: CatalogVersion
  download_path: string
}
