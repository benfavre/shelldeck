import { CanvasImage, Interactive, staticFile } from "remotion";

export const MediaStage: React.FC<{
  src: string;
  name: string;
  width?: number;
  height?: number;
  style?: React.CSSProperties;
  imageStyle?: React.CSSProperties;
  children?: React.ReactNode;
}> = ({
  src,
  name,
  width = 1920,
  height = 1080,
  style,
  imageStyle,
  children,
}) => {
  return (
    <Interactive.Div
      name={`${name} stage`}
      className="us-stage"
      style={{ position: "absolute", borderRadius: 28, ...style }}
    >
      <CanvasImage
        name={name}
        src={staticFile(`assets/${src}`)}
        width={width}
        height={height}
        fit="cover"
        style={{ width: "100%", height: "100%", ...imageStyle }}
      />
      {children}
    </Interactive.Div>
  );
};
