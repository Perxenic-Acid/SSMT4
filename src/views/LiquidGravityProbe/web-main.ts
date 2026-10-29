import { createApp } from 'vue'
import ElementPlus from 'element-plus'
import 'element-plus/dist/index.css'
import '../../styles/tokens.css'
import './web.css'
import LiquidGravityProbe from './LiquidGravityProbe.vue'

const params = new URLSearchParams(window.location.hash.replace(/^#/, ''))
const token = params.get('token') || ''
const game = params.get('game') || 'EFMI'

createApp(LiquidGravityProbe, {
  embeddedUrl: `${window.location.origin}/`,
  embeddedToken: token,
  embeddedGame: game,
}).use(ElementPlus).mount('#app')
