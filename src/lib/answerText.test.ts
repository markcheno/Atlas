import { describe, expect, it } from 'vitest';
import { paragraphs, type Piece } from './answerText';

const t = (text: string, bold = false, italic = false): Piece => ({
  kind: 'text',
  text,
  bold,
  italic,
});
const c = (n: number): Piece => ({ kind: 'cite', n });

describe('answer text', () => {
  it('splits paragraphs at blank lines and keeps line breaks inside one', () => {
    expect(paragraphs('\n\nFirst line\nsecond.\n\n  \n\nNext.  ')).toEqual([
      [t('First line\nsecond.')],
      [t('Next.')],
    ]);
  });

  it('turns checked citations into pieces, beside punctuation', () => {
    expect(paragraphs('Rome fell [1][3]. Then (see [12]), it rose.')).toEqual([
      [t('Rome fell '), c(1), c(3), t('. Then (see '), c(12), t('), it rose.')],
    ]);
  });

  it('renders closed bold and italic, citations inside them too', () => {
    expect(paragraphs('The book is **Candide [4]** by *Voltaire*, _a satire_.')).toEqual([
      [
        t('The book is '),
        t('Candide ', true),
        c(4),
        t(' by '),
        t('Voltaire', false, true),
        t(', '),
        t('a satire', false, true),
        t('.'),
      ],
    ]);
  });

  it('leaves marks that are not emphasis as they are', () => {
    for (const text of [
      'still **streaming',
      'an *open mark',
      '2 * 3 * 4 = 24',
      'snake_case_name and file_name',
      '** not bold **',
    ]) {
      expect(paragraphs(text)).toEqual([[t(text)]]);
    }
  });

  it('never reads markup', () => {
    expect(paragraphs('<b>bold</b> <img src=x onerror=alert(1)> [x] [1')).toEqual([
      [t('<b>bold</b> <img src=x onerror=alert(1)> [x] [1')],
    ]);
  });
});
