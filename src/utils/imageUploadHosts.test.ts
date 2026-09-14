import { describe, expect, it } from 'vitest';
import { encodeExtraFields, parseExtraFields, resolveUploadTarget, UPLOAD_HOST_PRESETS } from './imageUploadHosts';

describe('imageUploadHosts', () => {
  it('defaults to nuuls and resolves presets by id', () => {
    expect(resolveUploadTarget(undefined).url).toBe('https://i.nuuls.com/upload');
    const cat = resolveUploadTarget({ preset: 'catbox' });
    expect(cat.formField).toBe('fileToUpload');
    expect(cat.extraFields).toEqual({ reqtype: 'fileupload' });
    expect(resolveUploadTarget({ preset: 'litterbox' }).extraFields.time).toBe('72h');
  });

  it('treats a foreign url without a preset as custom', () => {
    const t = resolveUploadTarget({ url: 'https://my.host/up', form_field: 'f', extra_fields: 'a=1&b=x%20y' });
    expect(t.url).toBe('https://my.host/up');
    expect(t.formField).toBe('f');
    expect(t.extraFields).toEqual({ a: '1', b: 'x y' });
  });

  it('round-trips extra fields', () => {
    const f = { reqtype: 'fileupload', time: '72h' };
    expect(parseExtraFields(encodeExtraFields(f))).toEqual(f);
    expect(parseExtraFields('')).toEqual({});
    expect(parseExtraFields('novalue')).toEqual({});
  });

  it('every preset posts over https', () => {
    for (const p of UPLOAD_HOST_PRESETS) expect(p.url.startsWith('https://')).toBe(true);
  });
});
