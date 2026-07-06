import type { StoreApi } from 'zustand'
import type { AppState } from './types'

export type AppStoreSet = StoreApi<AppState>['setState']
export type AppStoreGet = StoreApi<AppState>['getState']
