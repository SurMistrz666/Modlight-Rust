/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** True in `vite --mode fixtures`: pages come from fixtures/, actions are answered locally. */
  readonly MODLIGHT_FIXTURES: boolean;
}
