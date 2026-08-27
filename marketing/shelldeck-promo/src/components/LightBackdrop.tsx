import { AbsoluteFill, CanvasImage, staticFile } from "remotion";
import { palette } from "../theme";

export const LightBackdrop: React.FC<{ imageOpacity?: number }> = ({
  imageOpacity = 0.12,
}) => {
  return (
    <AbsoluteFill
      style={{
        backgroundColor: palette.canvas,
        overflow: "hidden",
      }}
    >
      <CanvasImage
        name="Natural light backdrop"
        src={staticFile("assets/natural-light.png")}
        width={1920}
        height={1080}
        fit="cover"
        style={{
          width: "100%",
          height: "100%",
          opacity: imageOpacity,
        }}
      />
      <AbsoluteFill
        style={{
          background:
            "radial-gradient(circle at 12% 20%, rgba(34, 191, 160, 0.13), transparent 30%), radial-gradient(circle at 87% 76%, rgba(24, 139, 214, 0.14), transparent 34%), linear-gradient(135deg, rgba(255,255,255,0.78), rgba(247,250,252,0.9))",
        }}
      />
      <AbsoluteFill
        style={{
          opacity: 0.22,
          backgroundImage:
            "radial-gradient(rgba(17, 102, 177, 0.24) 1px, transparent 1px)",
          backgroundSize: "34px 34px",
          maskImage:
            "linear-gradient(to bottom right, black, transparent 38%, transparent 68%, black)",
        }}
      />
    </AbsoluteFill>
  );
};
