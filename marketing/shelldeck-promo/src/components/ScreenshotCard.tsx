import { CanvasImage, Interactive, staticFile } from "remotion";
import { palette, softShadow } from "../theme";

export const ScreenshotCard: React.FC<{
  src: string;
  name: string;
  style?: React.CSSProperties;
}> = ({ src, name, style }) => {
  return (
    <Interactive.Div
      name={`${name} window`}
      style={{
        position: "absolute",
        overflow: "hidden",
        borderRadius: 24,
        border: `1px solid ${palette.border}`,
        backgroundColor: palette.surface,
        boxShadow: softShadow,
        ...style,
      }}
    >
      <CanvasImage
        name={name}
        src={staticFile(`assets/${src}`)}
        width={1920}
        height={1080}
        fit="cover"
        style={{ width: "100%", height: "100%" }}
      />
    </Interactive.Div>
  );
};
