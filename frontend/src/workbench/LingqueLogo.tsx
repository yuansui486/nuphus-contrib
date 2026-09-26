/** The chosen B / 逐流 mark; decorative when paired with the visible product name. */
export function LingqueLogo({ size = 26 }: { size?: number }) {
  return (
    <img
      src="./lingque.svg"
      alt=""
      aria-hidden="true"
      width={size}
      height={size}
      draggable={false}
      style={{ display: 'block', flexShrink: 0 }}
    />
  )
}
