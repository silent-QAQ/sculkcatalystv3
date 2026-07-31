import type {
  ActiveComposerToken,
  ComposerTokenKind,
  ComposerTokenReplacement,
  ComposerTrigger,
  ModelShortcutCommand,
  ReplaceComposerTokenOptions,
} from './types'

const TRIGGER_KINDS: Readonly<Record<ComposerTrigger, ComposerTokenKind>> = {
  '/': 'skill',
  '／': 'skill',
  '%': 'skill',
  '％': 'skill',
  '#': 'file',
  '＃': 'file',
  '@': 'agent',
  '＠': 'agent',
}

const isWhitespace = (character: string): boolean => /\s/u.test(character)
const isBang = (character: string): boolean => character === '!' || character === '！'
const isOpenBrace = (character: string): boolean => character === '{' || character === '｛'
const isCloseBrace = (character: string): boolean => character === '}' || character === '｝'
const isColon = (character: string): boolean => character === ':' || character === '：'

function triggerKind(character: string): ComposerTokenKind | undefined {
  return TRIGGER_KINDS[character as ComposerTrigger]
}

function trimRange(value: string, start = 0, end = value.length): [number, number] {
  while (start < end && isWhitespace(value[start]!)) start += 1
  while (end > start && isWhitespace(value[end - 1]!)) end -= 1
  return [start, end]
}

/**
 * Finds the autocomplete token immediately before the cursor.
 *
 * A trigger is active only at the start of the input or immediately after
 * whitespace. Whitespace ends a token, which keeps symbols inside URLs,
 * email addresses and ordinary code expressions from opening suggestions.
 */
export function findActiveComposerToken(
  value: string,
  cursor = value.length,
): ActiveComposerToken | null {
  if (!Number.isInteger(cursor) || cursor < 0 || cursor > value.length) return null

  let start = cursor
  while (start > 0 && !isWhitespace(value[start - 1]!)) start -= 1
  if (start === cursor) return null

  const trigger = value[start]!
  const kind = triggerKind(trigger)
  if (!kind) return null

  let end = cursor
  while (end < value.length && !isWhitespace(value[end]!)) end += 1

  return {
    kind,
    trigger: trigger as ComposerTrigger,
    query: value.slice(start + 1, cursor),
    range: { start, end },
  }
}

/**
 * Replaces an active token with a selected candidate and returns the next
 * textarea value/cursor. Candidate text is bare by default, so the original
 * ASCII or full-width trigger is retained.
 */
export function replaceComposerToken(
  value: string,
  token: ActiveComposerToken,
  candidateText: string,
  options: ReplaceComposerTokenOptions = {},
): ComposerTokenReplacement {
  const { start, end } = token.range
  if (
    !Number.isInteger(start)
    || !Number.isInteger(end)
    || start < 0
    || end < start
    || end > value.length
    || value[start] !== token.trigger
  ) {
    return { value, cursor: Math.min(Math.max(end, 0), value.length) }
  }

  const replacement = options.candidateIncludesTrigger
    ? candidateText
    : `${token.trigger}${candidateText}`
  const suffix = value.slice(end)
  const trailingSpace = options.appendSpace && (suffix.length === 0 || !isWhitespace(suffix[0]!))
    ? ' '
    : ''
  const nextValue = `${value.slice(0, start)}${replacement}${trailingSpace}${suffix}`

  return {
    value: nextValue,
    cursor: start + replacement.length + trailingSpace.length,
  }
}

/**
 * Parses a whole-input model shortcut such as `!{model}:{content}!`.
 * ASCII and full-width bangs, braces and colons can be mixed. The optional
 * braces around content are removed from the returned semantic content.
 */
export function parseModelShortcut(value: string): ModelShortcutCommand | null {
  const [commandStart, commandEnd] = trimRange(value)
  if (commandEnd - commandStart < 5) return null
  if (!isBang(value[commandStart]!) || !isBang(value[commandEnd - 1]!)) return null

  let position = commandStart + 1
  if (!isOpenBrace(value[position]!)) return null

  position += 1
  const modelStart = position
  while (position < commandEnd - 1 && !isCloseBrace(value[position]!)) position += 1
  if (position >= commandEnd - 1) return null

  const model = value.slice(modelStart, position).trim()
  if (!model) return null

  position += 1
  while (position < commandEnd - 1 && isWhitespace(value[position]!)) position += 1
  if (position >= commandEnd - 1 || !isColon(value[position]!)) return null

  const [contentStart, contentEnd] = trimRange(value, position + 1, commandEnd - 1)
  let semanticStart = contentStart
  let semanticEnd = contentEnd
  let contentWasBraced = false

  if (
    semanticEnd - semanticStart >= 2
    && isOpenBrace(value[semanticStart]!)
    && isCloseBrace(value[semanticEnd - 1]!)
  ) {
    contentWasBraced = true
    ;[semanticStart, semanticEnd] = trimRange(value, semanticStart + 1, semanticEnd - 1)
  }

  return {
    model,
    content: value.slice(semanticStart, semanticEnd),
    range: { start: commandStart, end: commandEnd },
    contentWasBraced,
  }
}
