import { Img, Interactive, staticFile } from "remotion";

export const FocusedCapture: React.FC<{
  src: string;
  name: string;
  label: string;
  sourceWidth?: number;
  sourceHeight?: number;
  zoom?: number;
  imagePosition?: "left" | "center" | "right";
  imageTop?: number;
  style?: React.CSSProperties;
  children?: React.ReactNode;
}> = ({
  src,
  name,
  label,
  sourceWidth = 1800,
  sourceHeight = 1000,
  zoom = 1,
  imagePosition = "center",
  imageTop = 0,
  style,
  children,
}) => {
  return (
    <Interactive.Div
      name={`${name} frame`}
      className="us-focus-frame"
      style={{ position: "absolute", borderRadius: 30, ...style }}
    >
      <Interactive.Div
        name={`${name} chrome`}
        className="us-focus-chrome"
        style={{ height: 58, display: "flex", alignItems: "center", gap: 10, padding: "0 22px" }}
      >
        <span className="us-window-dot" style={{ backgroundColor: "#ff786b" }} />
        <span className="us-window-dot" style={{ backgroundColor: "#ffc64f" }} />
        <span className="us-window-dot" style={{ backgroundColor: "#48c78e" }} />
        <span style={{ marginLeft: 12, color: "#65758a", fontSize: 17, fontWeight: 700 }}>
          {label}
        </span>
      </Interactive.Div>
      <div style={{ position: "relative", flex: 1, overflow: "hidden", backgroundColor: "#f8fafc" }}>
        <Img
          name={name}
          src={staticFile(`assets/${src}`)}
          width={sourceWidth}
          height={sourceHeight}
          style={{
            position: "absolute",
            top: imageTop,
            left: imagePosition === "left" ? 0 : imagePosition === "center" ? "50%" : undefined,
            right: imagePosition === "right" ? 0 : undefined,
            width: "auto",
            height: `${zoom * 100}%`,
            translate: imagePosition === "center" ? "-50% 0px" : undefined,
          }}
        />
        {children}
      </div>
    </Interactive.Div>
  );
};
