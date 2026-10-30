import { closePad } from '../../app/controls'
import { requestStream, setStage, stopStagePlayer } from '../../app/stage'
import { state } from '../../app/state'
import { shell } from '../../shell/layout'

export function bridgeStage(mode: 'live' | 'playback') {
  if (mode === 'playback') {
    stopStagePlayer()
    closePad()
    setStage('playing')
    return
  }
  if (state.screen === 'watch' && shell.view === 'live' && state.view === 'live') void requestStream()
}
