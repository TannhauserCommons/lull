// lull's own easter eggs (ADR-0081), as the `default` pack declares them: the same declarations any pack can make.
// A design's pack re-declares the ones it has a sound of its own for (render.mjs), so its sound wins.
// `sound` is filled in by render.mjs with the pack's file for that egg, when the pack has one.

export const EGGS = [
  { name: 'night-owl', when: [{ clock: { from: '03:00', to: '04:00' } }, { player: { playing: true } }], once: 'night' },
  { name: 'new-year', when: [{ clock: { date: '12-31', from: '23:59:50', to: '24:00', countdown: true } }], line: '{{count}}', once: 'year' },
  { name: 'new-year-hello', when: [{ clock: { date: '01-01', from: '00:00', to: '00:00:08' } }], line: 'Happy new year', once: 'year' },
  { name: 'birthday', when: [{ clock: { date: '10-31' } }], once: 'year' },
  {
    name: 'word',
    when: [{ words: { in: ['title'], any: ['moon', 'rain', 'star', 'sun', 'snow', 'night', 'sea', 'storm', 'fire', 'dream'] } }],
    once: 'song'
  },
  { name: 'still', when: [{ player: { playing: true, screen: 'now', idle: 180 } }], once: 'song' },
  { name: 'on-repeat', when: [{ repeat: { times: 10 } }], line: 'Ten times in a row', once: 'song' },
  { name: 'credits', when: [{ gesture: { on: 'brand', taps: 7, within: 3 } }], go: '#/credits', once: 'always' },
  { name: 'milestone', when: [{ milestone: { of: 'likes', at: [100, 500, 1000] } }], line: '{{count}} liked songs', for: 6, once: 'always' },
  { name: 'long-wait', when: [{ player: { status: ['finding', 'queued', 'buffering'], playing: false, held: 20 } }], instead: true, once: 'song' },
  { name: 'rare-boot', when: [{ event: { on: 'boot' } }, { chance: { one_in: 40 } }], instead: true, once: 'always' },
];
