<script setup lang="ts">
import { computed, ref } from 'vue'
import { Check, Copy } from 'lucide-vue-next'
import { writeClipboard } from '../../lib/clipboard'

const props = defineProps<{ content: string }>()

type ContentBlock =
  | { kind: 'text'; content: string }
  | { kind: 'code'; content: string; language: string }

const blocks = computed<ContentBlock[]>(() => {
  const result: ContentBlock[] = []
  const fence = /```([^\r\n`]*)\r?\n([\s\S]*?)```/g
  let cursor = 0
  let match: RegExpExecArray | null
  while ((match = fence.exec(props.content)) !== null) {
    if (match.index > cursor) result.push({ kind: 'text', content: props.content.slice(cursor, match.index) })
    result.push({ kind: 'code', language: match[1].trim(), content: match[2].replace(/\r?\n$/, '') })
    cursor = match.index + match[0].length
  }
  if (cursor < props.content.length) result.push({ kind: 'text', content: props.content.slice(cursor) })
  return result.length ? result : [{ kind: 'text', content: props.content }]
})

const copiedBlock = ref<number | null>(null)
let copiedTimer: number | undefined
async function copyCode(content: string, index: number) {
  try { await writeClipboard(content) } catch { return }
  copiedBlock.value = index
  if (copiedTimer) window.clearTimeout(copiedTimer)
  copiedTimer = window.setTimeout(() => { copiedBlock.value = null }, 1600)
}
</script>

<template>
  <div class="message-content">
    <template v-for="(block, index) in blocks" :key="index">
      <p v-if="block.kind === 'text'">{{ block.content }}</p>
      <section v-else class="code-block">
        <header>
          <span>{{ block.language || '代码' }}</span>
          <button type="button" :title="copiedBlock === index ? '已复制' : '复制代码'" @click="copyCode(block.content, index)">
            <Check v-if="copiedBlock === index" />
            <Copy v-else />
            {{ copiedBlock === index ? '已复制' : '复制' }}
          </button>
        </header>
        <pre><code>{{ block.content }}</code></pre>
      </section>
    </template>
  </div>
</template>

<style scoped>
.message-content{min-width:0;color:#bec6d1;font-size:12px;line-height:1.72}
.message-content p{margin:0;white-space:pre-wrap;overflow-wrap:anywhere}
.code-block{margin:10px 0;overflow:hidden;border:1px solid color-mix(in srgb,var(--accent) 14%,rgba(255,255,255,.12));border-radius:9px;background:color-mix(in srgb,var(--panel) 48%,transparent);box-shadow:0 10px 26px rgba(0,0,0,.1);backdrop-filter:blur(12px) saturate(120%)}
.code-block>header{height:31px;display:flex;align-items:center;justify-content:space-between;padding:0 10px;border-bottom:1px solid rgba(255,255,255,.09);background:rgba(255,255,255,.045)}
.code-block>header span{color:#697684;font-size:9px;font-weight:600;text-transform:uppercase}
.code-block button{height:24px;display:flex;align-items:center;gap:5px;padding:0 7px;border:0;border-radius:5px;color:#7f8b97;background:transparent;font-size:8px}
.code-block button:hover{color:#d6dde4;background:rgba(255,255,255,.05)}
.code-block button svg{width:12px}
.code-block pre{max-width:100%;margin:0;padding:13px 14px;overflow:auto;color:#c5e4db;background:transparent;font:10px/1.7 'Cascadia Code',Consolas,monospace;tab-size:2}
.code-block code{font:inherit;white-space:pre}
</style>
