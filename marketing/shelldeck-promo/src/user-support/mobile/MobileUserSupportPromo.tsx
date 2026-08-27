import { TransitionSeries, springTiming } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { slide } from "@remotion/transitions/slide";
import { MobileAssistScene } from "./MobileAssistScene";
import { MobileIntroScene } from "./MobileIntroScene";
import { MobileModeSwitchScene } from "./MobileModeSwitchScene";
import { MobileOutcomeScene } from "./MobileOutcomeScene";
import { MobileSupportScene } from "./MobileSupportScene";
import { MobileUserRequestScene } from "./MobileUserRequestScene";

export const MobileUserSupportPromo: React.FC = () => (
  <TransitionSeries>
    <TransitionSeries.Sequence
      durationInFrames={120}
      name="Mobile journey intro"
    >
      <MobileIntroScene />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition
      presentation={slide({ direction: "from-right" })}
      timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
    />
    <TransitionSeries.Sequence
      durationInFrames={150}
      name="Mobile user request"
    >
      <MobileUserRequestScene />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition
      presentation={fade()}
      timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
    />
    <TransitionSeries.Sequence durationInFrames={105} name="Mobile mode switch">
      <MobileModeSwitchScene />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition
      presentation={slide({ direction: "from-left" })}
      timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
    />
    <TransitionSeries.Sequence
      durationInFrames={150}
      name="Mobile support reply"
    >
      <MobileSupportScene />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition
      presentation={fade()}
      timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
    />
    <TransitionSeries.Sequence durationInFrames={150} name="Mobile AI assist">
      <MobileAssistScene />
    </TransitionSeries.Sequence>
    <TransitionSeries.Transition
      presentation={fade()}
      timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
    />
    <TransitionSeries.Sequence durationInFrames={120} name="Mobile outcome">
      <MobileOutcomeScene />
    </TransitionSeries.Sequence>
  </TransitionSeries>
);
