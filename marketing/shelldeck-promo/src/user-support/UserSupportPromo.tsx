import { TransitionSeries, springTiming } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { slide } from "@remotion/transitions/slide";
import { AssistResolveScene } from "./scenes/AssistResolveScene";
import { JourneyIntroScene } from "./scenes/JourneyIntroScene";
import { JourneyOutcomeScene } from "./scenes/JourneyOutcomeScene";
import { ModeSwitchScene } from "./scenes/ModeSwitchScene";
import { SupportTriageScene } from "./scenes/SupportTriageScene";
import { UserRequestScene } from "./scenes/UserRequestScene";

export const UserSupportPromo: React.FC = () => {
  return (
    <TransitionSeries
      style={{
        translate: "-1px 0px",
      }}
    >
      <TransitionSeries.Sequence durationInFrames={120} name="Journey intro">
        <JourneyIntroScene />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={slide({ direction: "from-right" })}
        timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
      />
      <TransitionSeries.Sequence durationInFrames={150} name="User request">
        <UserRequestScene />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={fade()}
        timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
      />
      <TransitionSeries.Sequence durationInFrames={105} name="Switch User to Support">
        <ModeSwitchScene />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={slide({ direction: "from-left" })}
        timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
      />
      <TransitionSeries.Sequence durationInFrames={150} name="Support triage">
        <SupportTriageScene />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={fade()}
        timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
      />
      <TransitionSeries.Sequence durationInFrames={150} name="AI assisted resolution">
        <AssistResolveScene />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={fade()}
        timing={springTiming({ config: { damping: 200 }, durationInFrames: 14 })}
      />
      <TransitionSeries.Sequence durationInFrames={120} name="Outcome">
        <JourneyOutcomeScene />
      </TransitionSeries.Sequence>
    </TransitionSeries>
  );
};
