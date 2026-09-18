import { describe, expect, it } from 'vitest';
import { formatBytes, formatCount, formatMs, renderInlineMarkdown } from './format.ts';

describe('format utilities', () => {
  it('formatBytes handles B / KB / MB / GB', () => {
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(2048)).toBe('2.0 KB');
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB');
    expect(formatBytes(2 * 1024 * 1024 * 1024)).toBe('2.00 GB');
  });

  it('formatMs handles μs / ms / s', () => {
    expect(formatMs(0.5)).toBe('500 μs');
    expect(formatMs(16.67)).toMatch(/16\.\d+ ms/);
    expect(formatMs(1500)).toMatch(/1\.50 s/);
  });

  it('formatCount uses K / M suffixes', () => {
    expect(formatCount(42)).toBe('42');
    expect(formatCount(1500)).toBe('1.5K');
    expect(formatCount(2_500_000)).toBe('2.5M');
  });

  it('renderInlineMarkdown escapes HTML and supports headings/code/bold', () => {
    const input = '## Title\n**bold** and `code` and <script>alert(1)</script>';
    const html = renderInlineMarkdown(input);
    expect(html).toContain('<h2>Title</h2>');
    expect(html).toContain('<strong>bold</strong>');
    expect(html).toContain('<code>code</code>');
    expect(html).not.toContain('<script>');
  });
});