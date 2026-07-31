export type ComposerTokenKind = 'skill' | 'file' | 'agent'

export type SkillTrigger = '/' | '／' | '%' | '％'
export type FileTrigger = '#' | '＃'
export type AgentTrigger = '@' | '＠'
export type ComposerTrigger = SkillTrigger | FileTrigger | AgentTrigger

/** A half-open range: start is inclusive and end is exclusive. */
export interface ComposerTextRange {
  start: number
  end: number
}

export interface ActiveComposerToken {
  kind: ComposerTokenKind
  trigger: ComposerTrigger
  /** Text entered after the trigger and before the cursor. */
  query: string
  /** Range occupied by the active token, including its trigger. */
  range: ComposerTextRange
}

export interface ComposerTokenReplacement {
  value: string
  /** Suggested collapsed selection after applying the replacement. */
  cursor: number
}

export interface ReplaceComposerTokenOptions {
  /** Set when candidateText already contains the desired trigger. */
  candidateIncludesTrigger?: boolean
  /** Adds one trailing space unless whitespace already follows the token. */
  appendSpace?: boolean
}

export interface ModelShortcutCommand {
  model: string
  content: string
  /** The command range after excluding leading and trailing whitespace. */
  range: ComposerTextRange
  /** Whether the content used one optional pair of outer braces. */
  contentWasBraced: boolean
}
