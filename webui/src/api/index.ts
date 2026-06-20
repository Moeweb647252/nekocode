export * from './types'
/** @deprecated Login UI not yet implemented; auth backend is mounted. */
export { login } from './auth'
export {
  createThread,
  listThreads,
  deleteThread,
  activateThread,
  getThread,
  updateThread,
} from './thread'
export { streamGenerate } from './generate'
/** @deprecated Not yet wired into any UI component. */
export { watchStream } from './generate'
export type { GenerateCallbacks } from './generate'
export {
  createWorkspace,
  listWorkspaces,
  getWorkspace,
  updateWorkspace,
  deleteWorkspace,
} from './workspace'
export { getDirs, listDir, getModels, probeMcp, listSkills } from './util'
export type { DirsResponse, ListDirEntry, McpProbeToolInfo, SkillInfo } from './util'
export { createMiddleware, listMiddlewares, updateMiddleware, deleteMiddleware } from './middleware'
