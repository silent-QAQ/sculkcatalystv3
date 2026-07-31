// 模拟 ACP Agent：JSON-RPC 2.0 over stdio（按行分帧），用于端到端验证 Agent 接入
const readline = require('readline')
const rl = readline.createInterface({ input: process.stdin })
const send = obj => process.stdout.write(JSON.stringify(obj) + '\n')

rl.on('line', line => {
  let msg
  try { msg = JSON.parse(line) } catch { return }
  const { id, method, params } = msg
  if (method === 'initialize') {
    send({ jsonrpc: '2.0', id, result: { protocolVersion: 1, agentCapabilities: { loadSession: false } } })
  } else if (method === 'session/new') {
    send({ jsonrpc: '2.0', id, result: { sessionId: 'sess-mock-1' } })
  } else if (method === 'session/prompt') {
    const sessionId = params.sessionId
    const pieces = ['你好，', '这里是 ', '模拟 ACP Agent。', '已通过 ', 'session/update ', '流式返回', '回复内容。']
    let i = 0
    const timer = setInterval(() => {
      if (i < pieces.length) {
        send({ jsonrpc: '2.0', method: 'session/update', params: { sessionId, update: { sessionUpdate: 'agent_message_chunk', content: { type: 'text', text: pieces[i] } } } })
        i++
      } else {
        clearInterval(timer)
        send({ jsonrpc: '2.0', id, result: { stopReason: 'end_turn' } })
      }
    }, 50)
  } else if (id !== undefined) {
    send({ jsonrpc: '2.0', id, error: { code: -32601, message: 'method not found' } })
  }
})
