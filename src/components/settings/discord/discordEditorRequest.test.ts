import { describe, expect, it, vi } from 'vitest';
import { requestDiscordEditor, subscribeDiscordEditorRequest, takeDiscordEditorRequest } from './discordEditorRequest';

describe('editor request', () => {
  it('keeps a request made before anyone listens, until taken once', () => {
    requestDiscordEditor('Big picture');
    expect(takeDiscordEditorRequest()).toEqual({ row: 'Big picture' });
    expect(takeDiscordEditorRequest()).toBeNull();
  });

  it('tells a mounted listener', () => {
    const heard = vi.fn();
    const stop = subscribeDiscordEditorRequest(heard);
    requestDiscordEditor();
    expect(heard).toHaveBeenCalledTimes(1);
    expect(takeDiscordEditorRequest()).toEqual({ row: undefined });
    stop();
    requestDiscordEditor();
    expect(heard).toHaveBeenCalledTimes(1);
    takeDiscordEditorRequest();
  });
});
