# React + TypeScript Coding Standards

## General
- React 19 with TypeScript strict mode
- Use functional components with hooks
- Max line width: 100
- 2-space indentation
- Single quotes, semicolons required

## Component Rules
- One component per file
- Use named exports (not default exports)
- Use `.tsx` extension for components
- Props interface: `{ComponentName}Props`

## State Management
- Use Zustand for global state
- Use React hooks (`useState`, `useEffect`) for local state
- No Redux (Zustand is preferred)

## Styling
- Use Tailwind CSS 4 for utility-first styling
- Use HeroUI v3 components when available
- Avoid inline styles

## API Calls
- Use Axios with typed responses
- API functions in `src/api/` directory
- One file per domain (e.g. `api/system.ts`, `api/device.ts`)

## Build
- Vite for build tooling
- Build output in project root `static/` directory
- Proxy API calls in dev mode via vite.config.ts

## Project Structure
```
frontend/
├── src/
│   ├── App.tsx
│   ├── main.tsx
│   ├── pages/       # Route pages
│   ├── components/  # Reusable components
│   ├── api/         # API client functions
│   ├── hooks/       # Custom hooks (useWebSocket, etc.)
│   └── stores/      # Zustand stores
└── vite.config.ts
```

## Tooling
- ESLint with `typescript-eslint` + `eslint-plugin-react-hooks`
- Prettier for code formatting
- Format before commit
