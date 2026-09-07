import { TransitionSeries, linearTiming } from "@remotion/transitions";
import { fade } from "@remotion/transitions/fade";
import { slide } from "@remotion/transitions/slide";
import { ClosingScene } from "./scenes/ClosingScene";
import { HeroScene } from "./scenes/HeroScene";
import { TerminalScene } from "./scenes/TerminalScene";
import { WorkflowScene } from "./scenes/WorkflowScene";

export const ShellDeckPromo: React.FC = () => {
  return (
    <TransitionSeries>
      <TransitionSeries.Sequence durationInFrames={120} name="Hero">
        <HeroScene />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={fade()}
        timing={linearTiming({ durationInFrames: 12 })}
      />
      <TransitionSeries.Sequence durationInFrames={150} name="Terminal">
        <TerminalScene />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={slide({ direction: "from-right" })}
        timing={linearTiming({ durationInFrames: 12 })}
      />
      <TransitionSeries.Sequence durationInFrames={150} name="Workflows">
        <WorkflowScene />
      </TransitionSeries.Sequence>
      <TransitionSeries.Transition
        presentation={fade()}
        timing={linearTiming({ durationInFrames: 12 })}
      />
      <TransitionSeries.Sequence durationInFrames={120} name="Closing">
        <ClosingScene />
      </TransitionSeries.Sequence>
    </TransitionSeries>
  );
};
