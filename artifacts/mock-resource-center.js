// 本地资源站管理页/上传/解析流通模拟器，仅用于浏览器端到端测试。
const http = require('http')
const { createHash, randomUUID } = require('crypto')

const port = Number(process.env.MOCK_RESOURCE_PORT || 8798)
const token = process.env.MOCK_RESOURCE_TOKEN || 'flow-test-admin-token'
const segments = ['cores', 'plugins', 'skins', 'bbmodels', 'ui-textures', 'skills', 'plugin-configs']
const kindBySegment = { cores: 'core', plugins: 'plugin', skins: 'skin', bbmodels: 'bbmodel', 'ui-textures': 'ui_texture', skills: 'skill', 'plugin-configs': 'plugin_config' }
const segmentByKind = Object.fromEntries(Object.entries(kindBySegment).map(([segment, kind]) => [kind, segment]))
const projects = Object.fromEntries(segments.map(segment => [segment, []]))
const versions = new Map()
const objects = new Map()

projects.plugins.push({
  slug: 'luckperms', name: 'LuckPerms', summary: '跨平台权限管理插件。', description: '主流权限与用户组管理插件。', author: 'LuckPerms', homepage: 'https://luckperms.net', repository: 'https://github.com/LuckPerms/LuckPerms', preview_url: '', license: 'MIT', plugin_category: 'mainstream', target_plugin: '', tags: ['权限', '主流'], color: '#f5b942', featured: true,
})

function json(res, status, value, headers = {}) {
  res.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8', ...headers })
  res.end(JSON.stringify(value))
}

function readBody(req) {
  return new Promise(resolve => {
    const chunks = []
    req.on('data', chunk => chunks.push(chunk))
    req.on('end', () => resolve(Buffer.concat(chunks)))
  })
}

function authorized(req) {
  return req.headers.authorization === `Bearer ${token}`
}

function projectView(segment, project) {
  const items = versions.get(`${segment}/${project.slug}`) || []
  const published = items.filter(item => item.status === 'published')
  return {
    ...project,
    kind: kindBySegment[segment],
    version_count: items.length,
    published_versions: published.length,
    latest_version: published.at(-1)?.version || null,
    downloads: items.reduce((total, item) => total + item.downloads, 0),
    minecraft_versions: [...new Set(items.flatMap(item => item.minecraft_versions))],
    channels: [...new Set(items.map(item => item.channel))],
    loaders: [...new Set(items.flatMap(item => item.loaders))],
  }
}

function summary() {
  const allVersions = [...versions.values()].flat()
  return {
    core_projects: projects.cores.length,
    plugin_projects: projects.plugins.length,
    skin_projects: projects.skins.length,
    bbmodel_projects: projects.bbmodels.length,
    ui_texture_projects: projects['ui-textures'].length,
    skill_projects: projects.skills.length,
    plugin_config_projects: projects['plugin-configs'].length,
    versions: allVersions.length,
    downloads: allVersions.reduce((total, item) => total + item.downloads, 0),
    published_versions: allVersions.filter(item => item.status === 'published').length,
    featured_projects: Object.values(projects).flat().filter(item => item.featured).length,
  }
}

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${port}`)
  if (req.method === 'GET' && url.pathname === '/api/health') return json(res, 200, { ok: true })
  if (req.method === 'GET' && url.pathname === '/api/catalog/summary') return json(res, 200, summary())

  if (req.method === 'POST' && url.pathname.startsWith('/api/catalog/') && !authorized(req)) {
    return json(res, 401, { error: 'resource administrator token is invalid' })
  }
  if (['PUT', 'PATCH', 'DELETE'].includes(req.method) && url.pathname.startsWith('/api/catalog/') && !authorized(req)) {
    return json(res, 401, { error: 'resource administrator token is invalid' })
  }
  if (req.method === 'POST' && url.pathname === '/api/catalog/admin/verify') {
    return json(res, 200, { authorized: true, protected: true, upload_max_bytes: 268435456 })
  }
  if (req.method === 'POST' && url.pathname === '/api/catalog/admin/upload') {
    const body = await readBody(req)
    const kind = url.searchParams.get('kind')
    const segment = segmentByKind[kind]
    const project = url.searchParams.get('project')
    const version = url.searchParams.get('version')
    const filename = url.searchParams.get('filename')
    if (!segment || !projects[segment].some(item => item.slug === project)) return json(res, 404, { error: 'project not found' })
    const objectPath = `/objects/${segment}/${project}/${version}/${filename}`
    objects.set(objectPath, body)
    return json(res, 200, { download_url: `http://127.0.0.1:${port}${objectPath}`, object_path: objectPath, filename, size: body.length, sha256: createHash('sha256').update(body).digest('hex') })
  }

  const catalogMatch = url.pathname.match(/^\/api\/catalog\/([^/]+)(?:\/([^/]+))?(?:\/versions(?:\/([^/]+))?)?$/)
  if (catalogMatch && segments.includes(catalogMatch[1])) {
    const [, segment, slug, versionId] = catalogMatch
    if (!slug && req.method === 'GET') return json(res, 200, projects[segment].map(item => projectView(segment, item)))
    if (!slug && req.method === 'POST') {
      const input = JSON.parse((await readBody(req)).toString('utf8'))
      projects[segment].push(input)
      return json(res, 200, projectView(segment, input))
    }
    const project = projects[segment].find(item => item.slug === slug)
    if (!project) return json(res, 404, { error: 'project not found' })
    const key = `${segment}/${slug}`
    if (url.pathname.includes('/versions')) {
      const items = versions.get(key) || []
      if (!versionId && req.method === 'GET') return json(res, 200, items)
      if (!versionId && req.method === 'POST') {
        const input = JSON.parse((await readBody(req)).toString('utf8'))
        const saved = { id: randomUUID(), project: slug, downloads: 0, ...input }
        items.push(saved); versions.set(key, items)
        return json(res, 200, saved)
      }
      const item = items.find(entry => entry.version === versionId)
      if (item && req.method === 'GET') return json(res, 200, item)
    }
    if (req.method === 'GET') return json(res, 200, projectView(segment, project))
  }

  if (req.method === 'GET' && url.pathname === '/api/v1/resolve') {
    const segment = segmentByKind[url.searchParams.get('kind')]
    const slug = url.searchParams.get('project')
    const project = projects[segment]?.find(item => item.slug === slug)
    const item = (versions.get(`${segment}/${slug}`) || []).filter(entry => entry.status === 'published').at(-1)
    if (!project || !item) return json(res, 404, { error: 'compatible published version not found' })
    return json(res, 200, { kind: kindBySegment[segment], project: projectView(segment, project), version: item, download_path: `/api/v1/download/${kindBySegment[segment]}/${slug}/${item.version}` })
  }
  const downloadMatch = url.pathname.match(/^\/api\/v1\/download\/([^/]+)\/([^/]+)\/([^/]+)$/)
  if (req.method === 'GET' && downloadMatch) {
    const segment = segmentByKind[downloadMatch[1]]
    const item = (versions.get(`${segment}/${downloadMatch[2]}`) || []).find(entry => entry.version === downloadMatch[3])
    if (!item) return json(res, 404, { error: 'version not found' })
    item.downloads += 1
    res.writeHead(307, { Location: item.download_url }); return res.end()
  }
  if (req.method === 'GET' && objects.has(url.pathname)) {
    const body = objects.get(url.pathname)
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': body.length, ETag: `"${createHash('sha256').update(body).digest('hex')}"` })
    return res.end(body)
  }
  if (req.method === 'GET' && url.pathname === '/api/openapi.json') return json(res, 200, { openapi: '3.1.0', info: { title: 'Mock Resource API', version: '0.4.0' } })
  json(res, 404, { error: 'not found' })
})

server.listen(port, '127.0.0.1', () => console.log(`mock resource center on ${port}`))
