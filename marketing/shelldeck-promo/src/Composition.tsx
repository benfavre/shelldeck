import { Composition, Folder } from "remotion";
import { ClosingScene } from "./scenes/ClosingScene";
import { HeroScene } from "./scenes/HeroScene";
import { TerminalScene } from "./scenes/TerminalScene";
import { WorkflowScene } from "./scenes/WorkflowScene";
import { ShellDeckPromo } from "./ShellDeckPromo";
import { UserSupportPromo } from "./user-support/UserSupportPromo";
import { MobileUserSupportPromo } from "./user-support/mobile/MobileUserSupportPromo";
import { AssistResolveScene } from "./user-support/scenes/AssistResolveScene";
import { JourneyIntroScene } from "./user-support/scenes/JourneyIntroScene";
import { JourneyOutcomeScene } from "./user-support/scenes/JourneyOutcomeScene";
import { ModeSwitchScene } from "./user-support/scenes/ModeSwitchScene";
import { SupportTriageScene } from "./user-support/scenes/SupportTriageScene";
import { UserRequestScene } from "./user-support/scenes/UserRequestScene";

export const ShellDeckCompositions: React.FC = () => {
  return (
    <>
      <Folder name="ShellDeck-Promo-Scenes">
        <Composition
          id="ShellDeck-Hero"
          component={HeroScene}
          durationInFrames={120}
          fps={30}
          width={1920}
          height={1080}
        />
        <Composition
          id="ShellDeck-Terminal"
          component={TerminalScene}
          durationInFrames={150}
          fps={30}
          width={1920}
          height={1080}
        />
        <Composition
          id="ShellDeck-Workflows"
          component={WorkflowScene}
          durationInFrames={150}
          fps={30}
          width={1920}
          height={1080}
        />
        <Composition
          id="ShellDeck-Closing"
          component={ClosingScene}
          durationInFrames={120}
          fps={30}
          width={1920}
          height={1080}
        />
      </Folder>
      <Composition
        id="ShellDeck-Light-Promo"
        component={ShellDeckPromo}
        durationInFrames={504}
        fps={30}
        width={1920}
        height={1080}
      />
      <Folder name="User-Support-Scenes">
        <Composition
          id="User-Support-Intro"
          component={JourneyIntroScene}
          durationInFrames={120}
          fps={30}
          width={1920}
          height={1080}
        />
        <Composition
          id="User-Request"
          component={UserRequestScene}
          durationInFrames={150}
          fps={30}
          width={1920}
          height={1080}
        />
        <Composition
          id="Support-Triage"
          component={SupportTriageScene}
          durationInFrames={150}
          fps={30}
          width={1920}
          height={1080}
        />
        <Composition
          id="User-To-Support"
          component={ModeSwitchScene}
          durationInFrames={105}
          fps={30}
          width={1920}
          height={1080}
        />
        <Composition
          id="Support-AI-Assist"
          component={AssistResolveScene}
          durationInFrames={150}
          fps={30}
          width={1920}
          height={1080}
        />
        <Composition
          id="User-Support-Outcome"
          component={JourneyOutcomeScene}
          durationInFrames={120}
          fps={30}
          width={1920}
          height={1080}
        />
      </Folder>
      <Composition
        id="ShellDeck-User-Support-Promo"
        component={UserSupportPromo}
        durationInFrames={725}
        fps={30}
        width={1920}
        height={1080}
      />
      <Composition
        id="ShellDeck-User-Support-Promo-Mobile"
        component={MobileUserSupportPromo}
        durationInFrames={725}
        fps={30}
        width={1080}
        height={1350}
      />
    </>
  );
};
