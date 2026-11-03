import {Composition} from 'remotion';
import {Demo} from './Demo';
import {FPS, DURATION, STAGE_W, STAGE_H} from './timeline';

export const RemotionRoot: React.FC = () => (
  <Composition
    id="Demo"
    component={Demo}
    durationInFrames={DURATION}
    fps={FPS}
    width={STAGE_W}
    height={STAGE_H}
  />
);
