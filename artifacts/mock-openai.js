// 本地 OpenAI 格式模拟上游，用于端到端验证模型接入功能
const http = require('http')
const server = http.createServer((req, res) => {
  if (req.method === 'GET' && req.url.startsWith('/v1/models')) {
    res.writeHead(200, { 'Content-Type': 'application/json' })
    res.end(JSON.stringify({ object: 'list', data: [{ id: 'mock-gpt-4o' }, { id: 'mock-gpt-mini' }] }))
    return
  }
  if (req.method === 'POST' && req.url.startsWith('/v1/chat/completions')) {
    let body = ''
    req.on('data', c => (body += c))
    req.on('end', () => {
      let parsed = {}
      try { parsed = JSON.parse(body) } catch {}
      const model = parsed.model || 'mock'
      if (parsed.stream) {
        res.writeHead(200, { 'Content-Type': 'text/event-stream' })
        const pieces = ['你好', '，我是', '模拟上游模型 ', model, '。已收到', '你的消息', '，流式', '输出验证', '成功。']
        let i = 0
        const timer = setInterval(() => {
          if (i < pieces.length) {
            res.write(`data: ${JSON.stringify({ choices: [{ delta: { content: pieces[i] } }] })}\n\n`)
            i++
          } else {
            res.write('data: [DONE]\n\n')
            clearInterval(timer)
            res.end()
          }
        }, 60)
      } else {
        res.writeHead(200, { 'Content-Type': 'application/json' })
        res.end(JSON.stringify({
          choices: [{ message: { role: 'assistant', content: 'Hello! Mock reply from ' + model } }],
          usage: { prompt_tokens: 12, completion_tokens: 19, total_tokens: 31 },
        }))
      }
    })
    return
  }
  res.writeHead(404); res.end('not found')
})
server.listen(9944, '127.0.0.1', () => console.log('mock upstream on 9944'))
