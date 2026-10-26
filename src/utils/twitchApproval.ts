// Follow and channel points ride on Twitch's Drops approval. Rust asks for a
// missing one itself (services/twitch_approval.rs) and answers APPROVAL_CLOSED
// when the person closed the sheet, which is not a failure worth a toast.

/** True when a Twitch action ended because the approval sheet was closed. */
export const isApprovalClosed = (err: unknown): boolean =>
  String(err instanceof Error ? err.message : err).includes('APPROVAL_CLOSED');
