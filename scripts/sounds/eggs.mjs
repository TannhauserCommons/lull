// lull's own easter eggs (ADR-0081), as the `default` pack declares them: the same declarations any pack can make.
// A design's pack re-declares the ones it has a sound of its own for (render.mjs), so its sound wins.
// `sound` is filled in by render.mjs with the pack's file for that egg, when the pack has one.

export const EGGS = [
  { name: 'night-owl', when: [{ clock: { from: '03:00', to: '04:00' } }, { player: { playing: true } }], once: 'night' },
  { name: 'new-year', when: [{ clock: { date: '12-31', from: '23:59:50', to: '24:00', countdown: true } }], line: '{{count}}', once: 'year' },
  { name: 'new-year-hello', when: [{ clock: { date: '01-01', from: '00:00', to: '00:00:08' } }], line: 'Happy new year', once: 'year' },
  { name: 'birthday', when: [{ clock: { date: '10-31' } }], once: 'year' },
  // one egg per idea, each with its word in a dozen languages (an author lists the forms they want: no language's
  // plurals are assumed); a theme styles :root[data-egg~="moon"] whatever language the title is in
  { name: 'moon', when: [{ words: { in: ['title'], any: ['moon', 'luna', 'lune', 'mond', 'maan', 'lua', 'måne', 'księżyc', 'луна', '月亮', '달', 'चाँद'] } }], once: 'song' },
  { name: 'rain', when: [{ words: { in: ['title'], any: ['rain', 'lluvia', 'pluie', 'regen', 'pioggia', 'chuva', 'regn', 'deszcz', 'дождь', 'yağmur', '雨', '비', 'बारिश'] } }], once: 'song' },
  { name: 'star', when: [{ words: { in: ['title'], any: ['star', 'stars', 'estrella', 'étoile', 'stern', 'stella', 'estrela', 'ster', 'stjärna', 'gwiazda', 'звезда', 'yıldız', '星', '별', 'तारा'] } }], once: 'song' },
  { name: 'sun', when: [{ words: { in: ['title'], any: ['sun', 'sol', 'soleil', 'sonne', 'sole', 'zon', 'słońce', 'солнце', 'güneş', '太陽', '太阳', '태양', 'सूरज'] } }], once: 'song' },
  { name: 'snow', when: [{ words: { in: ['title'], any: ['snow', 'nieve', 'neige', 'schnee', 'neve', 'sneeuw', 'snö', 'śnieg', 'снег', '雪', 'बर्फ़'] } }], once: 'song' },
  { name: 'night', when: [{ words: { in: ['title'], any: ['night', 'nights', 'noche', 'nuit', 'nacht', 'notte', 'noite', 'natt', 'noc', 'ночь', 'gece', '夜', '밤', 'रात'] } }], once: 'song' },
  { name: 'sea', when: [{ words: { in: ['title'], any: ['sea', 'ocean', 'mar', 'mer', 'meer', 'mare', 'zee', 'hav', 'morze', 'море', 'deniz', '海', '바다', 'समंदर'] } }], once: 'song' },
  { name: 'storm', when: [{ words: { in: ['title'], any: ['storm', 'storms', 'tormenta', 'tempête', 'sturm', 'tempesta', 'tempestade', 'burza', 'буря', 'fırtına', '嵐', '风暴', '폭풍', 'तूफ़ान'] } }], once: 'song' },
  { name: 'fire', when: [{ words: { in: ['title'], any: ['fire', 'fuego', 'feu', 'feuer', 'fuoco', 'fogo', 'vuur', 'eld', 'ogień', 'огонь', 'ateş', '火', 'आग'] } }], once: 'song' },
  { name: 'dream', when: [{ words: { in: ['title'], any: ['dream', 'dreams', 'sueño', 'rêve', 'traum', 'sogno', 'sonho', 'droom', 'dröm', 'мечта', 'rüya', '夢', '梦', '꿈', 'सपना'] } }], once: 'song' },
  { name: 'still', when: [{ player: { playing: true, screen: 'now', idle: 180 } }], once: 'song' },
  { name: 'on-repeat', when: [{ repeat: { times: 10 } }], line: 'Ten times in a row', once: 'song' },
  { name: 'credits', when: [{ gesture: { on: 'brand', taps: 7, within: 3 } }], go: '#/credits', once: 'always' },
  { name: 'milestone', when: [{ milestone: { of: 'likes', at: [100, 500, 1000] } }], line: '{{count}} liked songs', for: 6, once: 'always' },
  { name: 'long-wait', when: [{ player: { status: ['finding', 'queued', 'buffering'], playing: false, held: 20 } }], instead: true, once: 'song' },
  { name: 'rare-boot', when: [{ event: { on: 'boot' } }, { chance: { one_in: 40 } }], instead: true, once: 'always' },
];
