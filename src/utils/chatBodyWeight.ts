// The chat message body's font weight: the interface font's own chat weight
// (--chat-body-weight, set per face in themes) moved by Text weight
// (chat_design.font_weight, where 400 is Normal). The default look holds and
// Light or Bold move it; a face without that weight draws its nearest. The
// iPhone draws the same sum (ChatFont.bodyWeight).
export const chatBodyWeight = (textWeight: number | undefined): string => {
  const delta = (textWeight ?? 400) - 400;
  return `clamp(100, calc(var(--chat-body-weight, 300) ${delta < 0 ? '-' : '+'} ${Math.abs(delta)}), 900)`;
};
