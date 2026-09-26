/** Retain the upstream splash's model-download and progress controls. */
export function brandSplash(html: string): string {
  const logo = /<div class="logo">[\s\S]*?<\/div>/
  if (!logo.test(html))
    throw new Error('Lingque: upstream splash logo changed; review branding adapter')
  return html
    .replace(/<title>[^<]*<\/title>/, '<title>灵雀 Lingque</title>')
    .replace(
      logo,
      '<div class="logo"><img src="./lingque.svg" width="88" height="88" alt="灵雀 Lingque" draggable="false" /></div>',
    )
}
