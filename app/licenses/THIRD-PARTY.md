# Third-party software in lull for Windows and Linux

## slskd 0.26.2

lull bundles slskd as the Soulseek client of the hub on your computer.

- **On Linux, unmodified:** the release's `slskd-0.26.2-linux-x64.zip` (or `-linux-arm64.zip`), checked by sha256 when lull is built (`app/build.rs`).
- **On Windows, modified by lull (2026-11-05):** slskd opens a file it is downloading so that lull may read it while it arrives (`FileShare.Read` instead of `FileShare.None`), so lull can play a Soulseek song while slskd downloads it. Nothing else differs. Built from slskd's 0.26.2 tag with https://github.com/TannhauserCommons/lull/blob/main/app/slskd/share-for-reading.patch, by the recipe in https://github.com/TannhauserCommons/lull/tree/main/app/slskd, and published as the `slskd-0.26.2-lull.1` pre-release of lull. Each zip also holds the patch and a NOTICE.
- Project: https://github.com/slskd/slskd
- Copyright: JP Dillingham and the slskd contributors
- Licence: GNU Affero General Public License v3.0, in `slskd-LICENSE.txt` next to this file
- Source code of the version both are built from: https://github.com/slskd/slskd/tree/0.26.2

## yt-dlp 2026.10.26

lull bundles yt-dlp, unmodified; the hub on your computer uses it to play YouTube when you turn YouTube on in Settings › Sources. The hub may replace it with a newer official release ("Keep yt-dlp up to date"), kept in its data folder.

- Project: https://github.com/yt-dlp/yt-dlp
- Licence: The Unlicense (public domain). The standalone binaries also hold Python and other libraries under their own licences, which the project lists: https://github.com/yt-dlp/yt-dlp/blob/2026.10.26/THIRD_PARTY_LICENSES.txt
- Binaries: the release's `yt-dlp.exe` and `yt-dlp_linux`, checked by sha256 when lull is built (`app/build.rs`)

## Deno 2.9.8

lull bundles Deno, unmodified: the JavaScript runtime yt-dlp needs for YouTube.

- Project: https://github.com/denoland/deno
- Copyright: the Deno authors
- Licence: MIT (https://github.com/denoland/deno/blob/v2.9.8/LICENSE.md):

  > Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
  >
  > The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
  >
  > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
- Binaries: the release's `deno-x86_64-pc-windows-msvc.zip` and `deno-x86_64-unknown-linux-gnu.zip`, checked by sha256 when lull is built (`app/build.rs`)

## libopus 1.6.1

lull's native Classic window decodes Opus (YouTube's format) with libopus, built into the app from its source (through the `opusic-sys` crate, 0.7.6).

- Project: https://opus-codec.org/
- Copyright 2001-2023 Xiph.Org, Skype Limited, Octasic, Jean-Marc Valin, Timothy B. Terriberry, CSIRO, Gregory Maxwell, Mark Borgerding, Erik de Castro Lopo, Mozilla, Amazon
- Licence: BSD-3-Clause:

  > Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:
  >
  > - Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.
  > - Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.
  > - Neither the name of Internet Society, IETF or IETF Trust, nor the names of specific contributors, may be used to endorse or promote products derived from this software without specific prior written permission.
  >
  > THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
- Opus is subject to the royalty-free patent licences specified at https://datatracker.ietf.org/ipr/1524/ (Xiph.Org Foundation), https://datatracker.ietf.org/ipr/1914/ (Microsoft Corporation) and https://datatracker.ietf.org/ipr/1526/ (Broadcom Corporation).

## Spotifast (parts of its source)

lull's native Classic window (`app/src/desktop/classic_native`) adapts code from Spotifast: its tables of where each part of a Winamp skin sits, its readers for `region.txt`, `viscolor.txt` and `pledit.txt`, its bitmap-font folding, its spectrum analyser, and its drawing of the equaliser (curve and sliders) and playlist (frame and positions).

- Project: https://github.com/crmne/spotifast (commit 672d665)
- Copyright (c) 2026 Carmine Paolino
- Licence: MIT:

  > Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:
  >
  > The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.
  >
  > THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
