// What the herdr apps may ask a Claude session to do, declared once: the fields each request carries and their types.
// A request that breaks it is refused with what is wrong and what would be accepted, never guessed at.

const REQUEST_FIELDS = {
  rerun: { workflowId: 'string' },
  redeploy: { stage: 'string', product: 'string' },
  release: { ticket: 'string' },
  extend: { stage: 'string', days: 'number' },
  'create-sandbox': { product: 'string', branch: 'string' },
  'stop-sandbox': { stage: 'string' },
} as const

type Fields = typeof REQUEST_FIELDS
type FieldType<T> = T extends 'number' ? number : string

export type Request = {
  [K in keyof Fields]: { type: K; label: string } & { -readonly [F in keyof Fields[K]]: FieldType<Fields[K][F]> }
}[keyof Fields]

// What the session writes back to the app, in requests/results/<file>; `refused` means it never ran.
export type Outcome = { ok: boolean; state: 'done' | 'failed' | 'refused'; message: string; at: number }

export function checkRequest(raw: unknown): { request: Request } | { refusal: string } {
  const types = Object.keys(REQUEST_FIELDS)
  const value = (raw ?? {}) as Record<string, unknown>
  const type = value.type
  if (typeof type !== 'string' || !(type in REQUEST_FIELDS)) {
    return { refusal: `unknown request type ${JSON.stringify(type)}; allowed: ${types.join(', ')}` }
  }
  const fields: Record<string, string> = { label: 'string', ...REQUEST_FIELDS[type as keyof Fields] }
  for (const [name, kind] of Object.entries(fields)) {
    if (typeof value[name] !== kind || value[name] === '') {
      const wanted = Object.entries(fields).map(([n, k]) => `${n} (${k})`).join(', ')
      return { refusal: `${type} needs ${name} as a ${kind}, got ${JSON.stringify(value[name])}; it takes ${wanted}` }
    }
  }
  return { request: value as Request }
}
