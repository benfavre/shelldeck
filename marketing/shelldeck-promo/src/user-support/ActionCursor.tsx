import { Easing, Interactive, interpolate, useCurrentFrame } from "remotion";

export const ActionCursor: React.FC<{
  name: string;
  appearAt: number;
  clickAt: number;
  from: [number, number];
  to: [number, number];
  color?: string;
  relativeToParentCenter?: boolean;
}> = ({ name, appearAt, clickAt, from, to, color = "#168ee0", relativeToParentCenter = false }) => {
  const frame = useCurrentFrame();

  return (
    <Interactive.Div
      name={name}
      style={{
        position: "absolute",
        zIndex: 20,
        left: relativeToParentCenter ? "50%" : 0,
        top: relativeToParentCenter ? "50%" : 0,
        width: 34,
        height: 43,
        opacity: interpolate(frame, [appearAt, appearAt + 5, clickAt + 13, clickAt + 20], [0, 1, 1, 0], {
          extrapolateLeft: "clamp",
          extrapolateRight: "clamp",
        }),
        translate: interpolate(frame, [appearAt, clickAt], [`${from[0]}px ${from[1]}px`, `${to[0]}px ${to[1]}px`], {
          extrapolateLeft: "clamp",
          extrapolateRight: "clamp",
          easing: Easing.bezier(0.16, 1, 0.3, 1),
        }),
        scale: interpolate(frame, [clickAt - 2, clickAt, clickAt + 4], [1, 0.92, 1], {
          extrapolateLeft: "clamp",
          extrapolateRight: "clamp",
          easing: Easing.spring({ damping: 180 }),
          output: "perceptual-scale",
        }),
      }}
    >
      <span
        style={{
          position: "absolute",
          left: -11,
          top: -12,
          width: 30,
          height: 30,
          borderRadius: 999,
          border: `2px solid ${color}`,
          opacity: interpolate(frame, [clickAt - 2, clickAt, clickAt + 10], [0, 0.72, 0], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
          }),
          scale: interpolate(frame, [clickAt - 2, clickAt + 10], [0.35, 2], {
            extrapolateLeft: "clamp",
            extrapolateRight: "clamp",
            easing: Easing.bezier(0.16, 1, 0.3, 1),
            output: "perceptual-scale",
          }),
        }}
      />
      <svg
        width="34"
        height="43"
        viewBox="0 0 34 43"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        style={{
          position: "absolute",
          inset: 0,
          filter: "drop-shadow(0 5px 5px rgba(15, 29, 47, .28))",
        }}
      >
        <path
          d="M4 2.7V31.4L11.4 24.3L18.2 39.1L24.1 36.3L17.6 22.2L28.2 21.8L4 2.7Z"
          fill="#182333"
          stroke="white"
          strokeWidth="2.4"
          strokeLinejoin="round"
        />
      </svg>
    </Interactive.Div>
  );
};
