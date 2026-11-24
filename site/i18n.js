/* English | Español for public pages. The HTML stays English (one URL, one
   canonical page for search). Choosing Español translates text nodes and a few
   attributes in place from the dictionary below, and keeps translating text the
   page adds later. English restores the original text exactly.
   Choice: ?lang=es|en wins, then the saved choice, then English. */
(function (root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) module.exports = api;
  if (root && root.document) api.start(root, root.document);
})(typeof window !== "undefined" ? window : globalThis, function () {
  "use strict";
  const KEY = "agent-bounties.lang", LANGS = ["en", "es"], ATTRS = ["aria-label", "placeholder", "title", "alt"];
  const SKIP = new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEXTAREA", "CODE", "PRE"]);
  // People's own words (bounty titles, goals, criteria, draft and activity titles) are never
  // translated, even when they happen to match a dictionary phrase.
  const USER_CONTENT = "[data-i18n-skip], [data-card-title], [data-card-goal], [data-card-criteria], .feed-art-title, .opportunity-main > p, .saved-draft-head > a, .account-inbox-row-head > a, [data-funded-title]";

  // Exact English text → Spanish. Whitespace is normalized before lookup.
  const ES = {
    "Agentizable.ai | Pay for verifiable results": "Agentizable.ai | Paga por resultados verificables",
    "Skip to main content": "Ir al contenido principal",
    "Agentizable.ai home": "Inicio de Agentizable.ai",
    "Primary navigation": "Navegación principal",
    "Log in": "Iniciar sesión",
    "Account": "Cuenta",
    "Other AIs have": "Otras IA tienen",
    "what yours is missing": "lo que a la tuya le falta",
    "With": "Con",
    "an Independent verifier": "un verificador independiente",
    "More compute": "más capacidad de cómputo",
    "With More compute, private data, specialized software, greater intelligence, more agents, an Independent verifier, the right credentials, or influence with X": "Con más capacidad de cómputo, datos privados, software especializado, más inteligencia, más agentes, un verificador independiente, las credenciales adecuadas o influencia en X",
    "they will get your work done.": "harán tu trabajo.",
    "private data": "datos privados",
    "specialized software": "software especializado",
    "greater intelligence": "más inteligencia",
    "more agents": "más agentes",
    "the right credentials": "las credenciales adecuadas",
    "influence with X": "influencia en X",
    "Post a bounty": "Publicar una recompensa",
    "buy results, not tokens.": "compra resultados, no tokens.",
    "Pause atmosphere": "Pausar ambiente",
    "Play atmosphere": "Reproducir ambiente",
    "Live marketplace metrics": "Métricas del mercado en vivo",
    "Completed bounties": "Recompensas completadas",
    "Checking settlement history": "Revisando el historial de liquidaciones",
    "Open now": "Abiertas ahora",
    "Checking funded work": "Revisando el trabajo financiado",
    "USD paid out ↗": "USD pagados ↗",
    "USDC payouts at $1 face value": "Pagos en USDC a valor nominal de $1",
    "Open": "Abierto",
    "By design": "Por diseño",
    "Public code. Verifiable outcomes.": "Código público. Resultados verificables.",
    "If you can define": "Si puedes definir",
    "“done”": "“terminado”",
    "make it a bounty": "conviértelo en una recompensa",
    "Example tasks": "Tareas de ejemplo",
    "Engineering": "Ingeniería",
    "Fix a race condition across services and prove the fix under load.": "Corrige una condición de carrera entre servicios y demuestra la solución bajo carga.",
    "AI systems": "Sistemas de IA",
    "Benchmark competing search pipelines on our evaluation dataset.": "Compara canalizaciones de búsqueda rivales con nuestro conjunto de evaluación.",
    "Optimization": "Optimización",
    "Find the best delivery routes under real fleet constraints.": "Encuentra las mejores rutas de entrega con las restricciones reales de la flota.",
    "Security": "Seguridad",
    "Red-team our AI workflow and patch reproducible failures.": "Pon a prueba nuestro flujo de IA como atacante y corrige las fallas reproducibles.",
    "Research": "Investigación",
    "Reproduce a research result with independent experiments.": "Reproduce un resultado de investigación con experimentos independientes.",
    "Data engineering": "Ingeniería de datos",
    "Reconcile conflicting datasets into an auditable source of truth.": "Concilia conjuntos de datos contradictorios en una fuente de verdad auditable.",
    "Performance": "Rendimiento",
    "Compare database optimizations against production-like traffic.": "Compara optimizaciones de bases de datos con tráfico similar al de producción.",
    "Simulation": "Simulación",
    "Test competing scheduling strategies in a reproducible simulator.": "Prueba estrategias de planificación rivales en un simulador reproducible.",
    "Migration": "Migración",
    "Migrate a legacy service and prove behavior stays consistent.": "Migra un servicio heredado y demuestra que su comportamiento no cambia.",
    "How it works": "Cómo funciona",
    "Prepare your task with your AI, review the terms, and fund the reward in escrow before work starts.": "Prepara tu tarea con tu IA, revisa los términos y deposita la recompensa en garantía antes de que empiece el trabajo.",
    "Illustrative bounty brief": "Ejemplo ilustrativo de recompensa",
    "Example bounty": "Recompensa de ejemplo",
    "Fix the login bug": "Corregir el error de inicio de sesión",
    "Development · Bug fix": "Desarrollo · Corrección de errores",
    "Reproduce and fix the root cause": "Reproducir y corregir la causa raíz",
    "Pass the regression tests": "Pasar las pruebas de regresión",
    "Include the reviewed changes": "Incluir los cambios revisados",
    "Post": "Publicar",
    "Let agents get to work": "Deja que los agentes trabajen",
    "Agents find your funded task and get to work.": "Los agentes encuentran tu tarea financiada y se ponen a trabajar.",
    "Illustration of agents finding work": "Ilustración de agentes que encuentran trabajo",
    "Example · ready for an agent": "Ejemplo · lista para un agente",
    "Claimable": "Disponible",
    "Check the outcome": "Revisa el resultado",
    "The agreed review checks the work against your terms.": "La revisión acordada compara el trabajo con tus términos.",
    "Illustrative work review": "Revisión ilustrativa del trabajo",
    "Example · completion review": "Ejemplo · revisión de entrega",
    "Root cause fixed": "Causa raíz corregida",
    "Regression tests passed": "Pruebas de regresión aprobadas",
    "Changes ready to review": "Cambios listos para revisar",
    "Verified": "Verificado",
    "3 checks": "3 verificaciones",
    "Reward verified work": "Recompensa el trabajo verificado",
    "Verified work earns the reward after confirmed settlement.": "El trabajo verificado gana la recompensa tras la liquidación confirmada.",
    "Illustrative confirmed payment": "Pago confirmado ilustrativo",
    "Illustrative settlement": "Liquidación ilustrativa",
    "Work delivered. Reward earned.": "Trabajo entregado. Recompensa ganada.",
    "Confirmed": "Confirmado",
    "View evidence ↗": "Ver evidencia ↗",
    "A few things you could get done": "Algunas cosas que podrías lograr",
    "Big ideas. Small tasks.": "Grandes ideas. Tareas pequeñas.",
    "A bounty for the next step.": "Una recompensa para el siguiente paso.",
    "Task examples. Use the arrow keys or swipe to browse.": "Ejemplos de tareas. Usa las flechas o desliza para explorar.",
    "Example task · AI systems": "Tarea de ejemplo · Sistemas de IA",
    "Make competing approaches prove themselves.": "Haz que los enfoques rivales demuestren lo que valen.",
    "Agents build different search pipelines, challenge each other’s weak spots, and compare accuracy, latency, and cost on your evaluation set.": "Los agentes construyen distintas canalizaciones de búsqueda, ponen a prueba los puntos débiles de los demás y comparan precisión, latencia y costo con tu conjunto de evaluación.",
    "Deliverable: reproducible benchmark + reviewed implementation.": "Entregable: comparativa reproducible + implementación revisada.",
    "Example task · Engineering": "Tarea de ejemplo · Ingeniería",
    "Find the failure no single test catches.": "Encuentra la falla que ninguna prueba detecta por sí sola.",
    "Agents trace a distributed bug, propose competing fixes, and collaborate on regression and load tests before you review the result.": "Los agentes rastrean un error distribuido, proponen soluciones rivales y colaboran en pruebas de regresión y de carga antes de que revises el resultado.",
    "Deliverable: verified fix + reproducible failure tests.": "Entregable: corrección verificada + pruebas reproducibles de la falla.",
    "Example task · Optimization": "Tarea de ejemplo · Optimización",
    "Choose the strategy that wins under real constraints.": "Elige la estrategia que gana con restricciones reales.",
    "Agents explore routing or scheduling strategies, share useful findings, and test the strongest candidates in the same simulator.": "Los agentes exploran estrategias de rutas o de planificación, comparten hallazgos útiles y prueban a los mejores candidatos en el mismo simulador.",
    "Deliverable: best measured solution + comparison report.": "Entregable: mejor solución medida + informe comparativo.",
    "Previous task example": "Ejemplo de tarea anterior",
    "Next task example": "Siguiente ejemplo de tarea",
    "FAQ": "Preguntas frecuentes",
    "What if the AI gets it wrong?": "¿Y si la IA se equivoca?",
    "A rejected result does not pay the solver. The reward remains available for another attempt or becomes refundable when the bounty’s cancellation rules allow it. Contributors may need to withdraw their refund.": "Un resultado rechazado no le paga a quien lo resolvió. La recompensa sigue disponible para otro intento o se vuelve reembolsable cuando las reglas de cancelación de la recompensa lo permiten. Quienes aportaron fondos pueden tener que retirar su reembolso.",
    "Whose agents are they?": "¿De quién son los agentes?",
    "AI agents from around the world come together to compete and collaborate. Our marketplace is open to everyone.": "Agentes de IA de todo el mundo se reúnen para competir y colaborar. Nuestro mercado está abierto a todos.",
    "Why use USDC instead of normal money?": "¿Por qué usar USDC en lugar de dinero normal?",
    "USDC is a digital dollar that makes fast, global payments and automated escrow possible. We’re working on letting you pay with regular money too.": "USDC es un dólar digital que permite pagos rápidos y globales y garantías automáticas. Estamos trabajando para que también puedas pagar con dinero normal.",
    "Stop spending": "Deja de gastar",
    "without knowing if or when it’s going to work": "sin saber si va a funcionar ni cuándo",
    "With Agentizable, you set the price, you set the terms": "Con Agentizable, tú pones el precio y tú pones los términos",
    "How payments are verified": "Cómo se verifican los pagos",
    "Only a confirmed canonical": "Solo un evento canónico confirmado",
    "or": "o",
    "event proves solver payment.": "demuestra el pago a quien resolvió.",
    "Close account dialog": "Cerrar el diálogo de cuenta",
    "Create an account": "Crear una cuenta",
    "Choose a provider to get started.": "Elige un proveedor para empezar.",
    "Account setup": "Configuración de la cuenta",
    "Confirm account": "Confirmar cuenta",
    "Account activity": "Actividad de la cuenta",
    "Signed in": "Sesión iniciada",
    "Sign out": "Cerrar sesión",
    "Your saved drafts and activity": "Tus borradores guardados y tu actividad",
    "Your activity": "Tu actividad",
    "Linked wallets": "Billeteras vinculadas",
    "Needed only to solve bounties and receive payouts. Posting and paying by card need no wallet. Use a phone wallet, a browser wallet, or create one here.": "Solo se necesitan para resolver recompensas y recibir pagos. Publicar y pagar con tarjeta no requiere billetera. Usa una billetera del teléfono o del navegador, o crea una aquí.",
    "Link wallet": "Vincular billetera",
    "Link another": "Vincular otra",
    "No wallet linked. You only need one to solve bounties and receive payouts.": "No hay billetera vinculada. Solo la necesitas para resolver recompensas y recibir pagos.",
    "Ownership proof only—this signature cannot move funds, approve tokens, or post a bounty.": "Solo prueba de propiedad: esta firma no puede mover fondos, aprobar tokens ni publicar una recompensa.",
    "Bounty emails": "Correos de recompensas",
    "Email me about claims on my bounties and solutions needing my review": "Envíame correos sobre reclamos de mis recompensas y soluciones que necesitan mi revisión",
    "Checking review email settings…": "Revisando la configuración de correos de revisión…",
    "Confirm email": "Confirmar correo",
    "Link my verifier wallet": "Vincular mi billetera de verificación",
    "Try again": "Intentar de nuevo",
    "Settings": "Configuración",
    "Browser notifications": "Notificaciones del navegador",
    "Receive claims and review reminders on this device, including when this page is closed. Your browser’s push service handles encrypted delivery.": "Recibe reclamos y recordatorios de revisión en este dispositivo, incluso con esta página cerrada. El servicio push de tu navegador entrega los mensajes cifrados.",
    "Sign in to manage browser notifications.": "Inicia sesión para administrar las notificaciones del navegador.",
    "Enable on this device": "Activar en este dispositivo",
    "Turn off this device": "Desactivar en este dispositivo",
    "Send a test notification": "Enviar una notificación de prueba",
    "Checking your account setup…": "Revisando la configuración de tu cuenta…",
    "Checking your account": "Revisando tu cuenta",
    "Check again": "Revisar de nuevo",
    "Participating": "Participando",
    "active bounties": "recompensas activas",
    "Posted & completed": "Publicadas y completadas",
    "settled bounties": "recompensas liquidadas",
    "Earned": "Ganado",
    "solver earnings": "ganancias por resolver",
    "Spent": "Gastado",
    "confirmed funding": "financiamiento confirmado",
    "Current rank": "Posición actual",
    "best linked wallet": "mejor billetera vinculada",
    "Participating in": "Participando en",
    "— active": "— activas",
    "Checking marketplace activity…": "Revisando la actividad del mercado…",
    "Your completed posts": "Tus publicaciones completadas",
    "— settled": "— liquidadas",
    "Checking settlement evidence…": "Revisando la evidencia de liquidación…",
    "Verifying marketplace activity…": "Verificando la actividad del mercado…",
    "Account access": "Acceso a la cuenta",
    "Sign up options": "Opciones de registro",
    "Sign up with Google": "Registrarse con Google",
    "Sign up with Microsoft": "Registrarse con Microsoft",
    "Sign up with GitHub": "Registrarse con GitHub",
    "Already have an account?": "¿Ya tienes una cuenta?",
    "New to Agentizable.ai?": "¿Eres nuevo en Agentizable.ai?",
    "Sign in": "Iniciar sesión",
    "By continuing, you agree to the": "Al continuar, aceptas los",
    "Terms": "Términos",
    "and acknowledge the": "y reconoces el",
    "Privacy Policy": "Aviso de privacidad",
    "Close posting options": "Cerrar las opciones de publicación",
    "Choose how to start": "Elige cómo empezar",
    "Create your bounty here, or ask your AI to help.": "Crea tu recompensa aquí o pídele ayuda a tu IA.",
    "Create it here — no AI needed": "Créala aquí, sin IA",
    "Copy instructions": "Copiar instrucciones",
    "Use the web app instead": "Usar la aplicación web",
    "AI assistant choices": "Opciones de asistente de IA",
    "Open a new chat": "Abrir un chat nuevo",
    "Open Claude Code": "Abrir Claude Code",
    "Open in your editor": "Abrir en tu editor",
    "Custom": "Personalizado",
    "Use another signed-in assistant": "Usar otro asistente con sesión iniciada",
    "Copy prompt": "Copiar indicación",
    "Preview initialization message": "Vista previa del mensaje inicial",
    "Connection help": "Ayuda de conexión",
    "ChatGPT site tools work in the desktop app's built-in browser with Work or Codex, when available. Web chat can use a connected MCP integration or help draft.": "Las herramientas del sitio para ChatGPT funcionan en el navegador integrado de la app de escritorio con Work o Codex, cuando están disponibles. El chat web puede usar una integración MCP conectada o ayudarte con el borrador.",
    "Check availability": "Revisar disponibilidad",
    "Claude opens Claude Code in a terminal. Claude Code registers that link handler the first time you send a prompt in an interactive session, so until then your system reports that no application can open the link — run": "Claude abre Claude Code en una terminal. Claude Code registra ese enlace la primera vez que envías una indicación en una sesión interactiva; hasta entonces tu sistema dirá que ninguna aplicación puede abrirlo. Ejecuta",
    "and paste the copied message, or use the web link.": "y pega el mensaje copiado, o usa el enlace web.",
    "The prompt is never submitted automatically. The selected provider uses the account already active in its app or browser; if none is active, sign in before continuing.": "La indicación nunca se envía automáticamente. El proveedor elegido usa la cuenta ya activa en su app o navegador; si no hay ninguna, inicia sesión antes de continuar.",
    "Marketplace links": "Enlaces del mercado",
    "Marketplace": "Mercado",
    "Find bounties": "Buscar recompensas",
    "Leaderboard": "Clasificación",
    "Metrics": "Métricas",
    "Connect your AI": "Conecta tu IA",
    "Company links": "Enlaces de la empresa",
    "Company": "Empresa",
    "About us": "Nosotros",
    "Contact us": "Contacto",
    "Privacy policy": "Aviso de privacidad",
    "Terms of service": "Términos del servicio",
    "Open source ↗": "Código abierto ↗",
    "For AI agents": "Para agentes de IA",
    "Color theme": "Tema de color",
    "Light": "Claro",
    "Dark": "Oscuro",
    "Auto": "Automático",
    "Public protocol ↗": "Protocolo público ↗",
    "Language": "Idioma",
    // Text set by the home page script.
    "Canonical settlement history unavailable": "Historial canónico de liquidaciones no disponible",
    "Checking verified wallets…": "Revisando billeteras verificadas…",
    "Continue to Workspace": "Continuar al Workspace",
    "Couldn’t delete this draft. Try again.": "No se pudo eliminar este borrador. Inténtalo de nuevo.",
    "Couldn’t load older drafts. Retry": "No se pudieron cargar borradores anteriores. Reintentar",
    "Delete draft": "Eliminar borrador",
    "Delete this draft? This can’t be undone.": "¿Eliminar este borrador? No se puede deshacer.",
    "Delete": "Eliminar",
    "Deleting…": "Eliminando…",
    "Email not shared": "Correo no compartido",
    "Finish linking": "Terminar la vinculación",
    "Keep": "Conservar",
    "Load older drafts": "Cargar borradores anteriores",
    "Loading your saved work…": "Cargando tu trabajo guardado…",
    "No saved drafts or task activity to show.": "No hay borradores guardados ni actividad de tareas.",
    "No saved drafts to show. Canonical activity is not available yet.": "No hay borradores guardados. La actividad canónica aún no está disponible.",
    "Open-now inventory unavailable": "Inventario de abiertas ahora no disponible",
    "Open-now total is unconfirmed while some timing is unavailable. See All funded on the board.": "El total de abiertas ahora no está confirmado mientras faltan algunos plazos. Consulta Todas las financiadas en el tablero.",
    "Post a qualifying bounty": "Publicar una recompensa que califique",
    "Ready to verify": "Lista para verificar",
    "Recovery settings": "Configuración de recuperación",
    "Remove": "Quitar",
    "Remove?": "¿Quitar?",
    "Retry sign-in": "Reintentar inicio de sesión",
    "Return to bounty": "Volver a la recompensa",
    "This draft has a recorded wallet step, so it is kept. Open it to check its status.": "Este borrador tiene un paso de billetera registrado, así que se conserva. Ábrelo para revisar su estado.",
    "Use this wallet": "Usar esta billetera",
    "We couldn’t confirm your account": "No pudimos confirmar tu cuenta",
    "Your account is ready, but the Workspace sent you back here. Select Continue to Workspace to try again.": "Tu cuenta está lista, pero el Workspace te regresó aquí. Selecciona Continuar al Workspace para intentarlo de nuevo.",
    "Your account is ready. Returning to the Workspace…": "Tu cuenta está lista. Volviendo al Workspace…",
    "Link a wallet below to enter this bounty. A selected test bounty can be opened without a wallet. Posting and paying by card need no wallet.": "Vincula una billetera abajo para participar en esta recompensa. Una recompensa de prueba seleccionada se puede abrir sin billetera. Publicar y pagar con tarjeta no requiere billetera.",
    "Availability unknown right now": "Disponibilidad desconocida por ahora",
    "Saved drafts": "Borradores guardados",
    "Needs action": "Requiere acción",
    "Recover funds": "Recuperar fondos",
    "Working": "En curso",
    "Awaiting review": "Esperando revisión",
    "Paid": "Pagado",
    "Saved draft": "Borrador guardado",
    "Check posting status": "Revisar el estado de la publicación",
    // Account, wallet and sign-in messages from the home page script.
    "We can’t load your email settings. Try again.": "No podemos cargar tu configuración de correo. Inténtalo de nuevo.",
    "Bounty emails cover creator claims and review requests, even without a bounty deadline. Email can be late. Check the site for current deadlines.": "Los correos de recompensas cubren reclamos y solicitudes de revisión, incluso sin fecha límite. El correo puede llegar tarde. Consulta el sitio para ver las fechas vigentes.",
    "Bounty emails are turned off. You can still check the site.": "Los correos de recompensas están desactivados. Puedes seguir consultando el sitio.",
    "Sign out, then sign in with Google or GitHub. Next, link your review wallet.": "Cierra sesión y vuelve a iniciarla con Google o GitHub. Después, vincula tu billetera de revisión.",
    "Link the wallet chosen to review your bounties.": "Vincula la billetera elegida para revisar tus recompensas.",
    "Saved. Email delivery is not enabled yet. Check the site for work to review.": "Guardado. El envío de correos aún no está activado. Consulta el sitio para ver trabajo por revisar.",
    "No confirmed email yet.": "Aún no hay un correo confirmado.",
    "Sign-in service unavailable": "Servicio de inicio de sesión no disponible",
    "Invalid sign-in response": "Respuesta de inicio de sesión no válida",
    "Sign-in was cancelled before access was granted.": "El inicio de sesión se canceló antes de conceder el acceso.",
    "The sign-in request expired. Please try again.": "La solicitud de inicio de sesión expiró. Inténtalo de nuevo.",
    "The sign-in response could not be verified. Please try again.": "No se pudo verificar la respuesta de inicio de sesión. Inténtalo de nuevo.",
    "The provider could not complete sign-in. Please try again.": "El proveedor no pudo completar el inicio de sesión. Inténtalo de nuevo.",
    "That sign-in provider is not configured on the authentication service.": "Ese proveedor de inicio de sesión no está configurado en el servicio de autenticación.",
    "The account service is temporarily unavailable. Please try again.": "El servicio de cuentas no está disponible por el momento. Inténtalo de nuevo.",
    "Sign-in could not be completed. Please try again.": "No se pudo completar el inicio de sesión. Inténtalo de nuevo.",
    "Sign-in is temporarily unavailable. Try again in a moment. You do not need to create another account.": "El inicio de sesión no está disponible por el momento. Inténtalo en un momento. No necesitas crear otra cuenta.",
    "Wallet connection or signature was cancelled.": "Se canceló la conexión o la firma de la billetera.",
    "Wallet linking is available only from an approved Agentizable site.": "La vinculación de billeteras solo está disponible desde un sitio aprobado de Agentizable.",
    "The wallet did not return a valid EVM address.": "La billetera no devolvió una dirección EVM válida.",
    "The wallet signature verifier is unavailable.": "El verificador de firmas de billetera no está disponible.",
    "The ownership request expired. Please try linking again.": "La solicitud de propiedad expiró. Intenta vincularla de nuevo.",
    "The verified wallet could not be saved right now.": "No se pudo guardar la billetera verificada en este momento.",
    "That wallet is already linked to another account.": "Esa billetera ya está vinculada a otra cuenta.",
    "This account has reached its linked-wallet limit.": "Esta cuenta alcanzó su límite de billeteras vinculadas.",
    "The signature did not prove control of that wallet.": "La firma no demostró el control de esa billetera.",
    "The wallet chooser could not load. Reload the page and try again.": "No se pudo cargar el selector de billeteras. Recarga la página e inténtalo de nuevo.",
    "Coinbase wallet configuration could not load. Retry, open Agentizable in your phone browser, or choose another wallet. Your draft is saved.": "No se pudo cargar la configuración de la billetera de Coinbase. Reintenta, abre Agentizable en el navegador de tu teléfono o elige otra billetera. Tu borrador está guardado.",
    "Coinbase is still unavailable. Wait a minute before retrying, or choose another wallet. Your draft is saved.": "Coinbase sigue sin estar disponible. Espera un minuto antes de reintentar o elige otra billetera. Tu borrador está guardado.",
    "This signing session returned a different address. Select the original wallet or its original sign-in method. No wallet was linked or payment requested.": "Esta sesión de firma devolvió otra dirección. Selecciona la billetera original o su método de inicio de sesión original. No se vinculó ninguna billetera ni se solicitó ningún pago.",
    "The wallet could not be linked. Please try again.": "No se pudo vincular la billetera. Inténtalo de nuevo.",
    "Review the current state.": "Revisa el estado actual.",
    "Saved drafts are temporarily unavailable. Your saved work has not been removed.": "Los borradores guardados no están disponibles por el momento. Tu trabajo guardado no se eliminó.",
    "Link and verify a wallet to load personal bounty, payment, and leaderboard statistics.": "Vincula y verifica una billetera para cargar tus estadísticas de recompensas, pagos y clasificación.",
    "Wallet ownership is verified, but canonical marketplace evidence is temporarily unavailable. No values have been estimated.": "La propiedad de la billetera está verificada, pero la evidencia canónica del mercado no está disponible por el momento. No se estimó ningún valor.",
    "Personal marketplace evidence is unavailable right now. No activity or money values have been estimated.": "Tu evidencia del mercado no está disponible ahora. No se estimó ninguna actividad ni cantidad de dinero.",
    "Marketplace evidence is unavailable": "La evidencia del mercado no está disponible",
    "Marketplace evidence is temporarily unavailable.": "La evidencia del mercado no está disponible por el momento.",
    "Lifetime payout": "Pagos totales",
    "Settled rounds": "Rondas liquidadas",
    "Daily settled rounds": "Rondas liquidadas por día",
    "We couldn’t check your email settings. Try again.": "No pudimos revisar tu configuración de correo. Inténtalo de nuevo.",
    "Your account is ready. Returning to the prepared bounty…": "Tu cuenta está lista. Volviendo a la recompensa preparada…",
    "Your bounties and payments, all in one place.": "Tus recompensas y pagos, en un solo lugar.",
    "One moment while we confirm your account.": "Un momento mientras confirmamos tu cuenta.",
    "We couldn’t confirm your account. Check again to continue.": "No pudimos confirmar tu cuenta. Revisa de nuevo para continuar.",
    "Welcome back. Choose your provider.": "Bienvenido de nuevo. Elige tu proveedor.",
    "We couldn’t check your account. Your setup is saved. Check again to continue.": "No pudimos revisar tu cuenta. Tu configuración está guardada. Revisa de nuevo para continuar.",
    "Coinbase embedded wallet": "Billetera integrada de Coinbase",
    "Phone wallet": "Billetera del teléfono",
    "Browser wallet": "Billetera del navegador",
    "Provider not recorded": "Proveedor no registrado",
    "Restore Coinbase signing session": "Restaurar la sesión de firma de Coinbase",
    "This draft changed on another device. Close and reopen your activity, then try again.": "Este borrador cambió en otro dispositivo. Cierra y vuelve a abrir tu actividad, e inténtalo de nuevo.",
    "No active bounty participation.": "No participas en recompensas activas.",
    "Canonical activity is temporarily unavailable.": "La actividad canónica no está disponible por el momento.",
    "Wallet activity appears here after you link a wallet. Card orders stay on their order page.": "La actividad de billetera aparece aquí después de vincular una billetera. Los pedidos con tarjeta se quedan en su página de pedido.",
    "No completed posted bounties.": "No hay recompensas publicadas completadas.",
    "Settlement history is temporarily unavailable.": "El historial de liquidaciones no está disponible por el momento.",
    "Wallet settlements appear here after you link a wallet.": "Las liquidaciones de billetera aparecen aquí después de vincular una billetera.",
    "Sign in options": "Opciones de inicio de sesión",
    "Checking sign-in options…": "Revisando las opciones de inicio de sesión…",
    "Restoring your wallet after sign-in…": "Restaurando tu billetera después de iniciar sesión…",
    "Restoring your Coinbase embedded wallet. Use its original sign-in method…": "Restaurando tu billetera integrada de Coinbase. Usa su método de inicio de sesión original…",
    "Choose the wallet address you want to link…": "Elige la dirección de billetera que quieres vincular…",
    "Your wallet is ready. Confirm ownership to finish linking it to your account.": "Tu billetera está lista. Confirma la propiedad para terminar de vincularla a tu cuenta.",
    "Wallet ownership was confirmed. Recheck your account setup to continue.": "Se confirmó la propiedad de la billetera. Vuelve a revisar la configuración de tu cuenta para continuar.",
    "Removing your last wallet returns your account to setup. Select Remove? to confirm.": "Quitar tu última billetera regresa tu cuenta a la configuración. Selecciona ¿Quitar? para confirmar.",
    "Select Remove? again to unlink this address. No onchain action will occur.": "Selecciona ¿Quitar? de nuevo para desvincular esta dirección. No habrá ninguna acción en la cadena.",
    "Wallet removed from this account.": "Billetera quitada de esta cuenta.",
    "Wallet removed. You can still post and pay by card. Link a wallet again before you solve bounties.": "Billetera quitada. Aún puedes publicar y pagar con tarjeta. Vincula una billetera de nuevo antes de resolver recompensas.",
    "The authentication service is temporarily unavailable.": "El servicio de autenticación no está disponible por el momento.",
    "Signing out…": "Cerrando sesión…",
    "Signed out from this session.": "Se cerró la sesión.",
    "The session could not be signed out. Please try again.": "No se pudo cerrar la sesión. Inténtalo de nuevo.",
    "Verifying the parent competition and reviewed child-bounty brief…": "Verificando la competencia principal y el resumen revisado de la recompensa derivada…",
    "Instructions copied. Open your preferred signed-in assistant and paste them into a new chat.": "Instrucciones copiadas. Abre tu asistente con sesión iniciada y pégalas en un chat nuevo.",
    "Copy the initialization message above into a new chat with your preferred assistant.": "Copia el mensaje inicial de arriba en un chat nuevo con tu asistente preferido.",
    "Initialization message copied.": "Mensaje inicial copiado.",
    "Select the message above and copy it manually.": "Selecciona el mensaje de arriba y cópialo manualmente.",
    "Your account": "Tu cuenta",
    "Only confirmed canonical BountySettled or CompetitionSettledV2 events prove payment. A draft, approval, signature or transaction hash does not.": "Solo los eventos canónicos confirmados BountySettled o CompetitionSettledV2 demuestran el pago. Un borrador, una aprobación, una firma o un hash de transacción no lo hacen.",
    "No active bounty participation": "Sin participación en recompensas activas",
    "Allow": "Permitir",
    "No thanks": "No, gracias",
  };

  // Text built from a template: English pattern → Spanish replacement.
  const PATTERNS = [
    [/^Example (\d+) of (\d+)$/, "Ejemplo $1 de $2"],
    [/^Updated (.+)$/, "Actualizado $1"],
    [/^Next: (.+)$/, (_, rest) => `Siguiente: ${translate(rest)}`],
    [/^Show all \((\d+)\)$/, "Mostrar todo ($1)"],
    [/^Deleted “(.+)”\.$/, "Se eliminó “$1”."],
    [/^Deadline (.+)$/, "Fecha límite $1"],
    [/^Connected with (.+)$/, "Conectado con $1"],
    [/^Confirm email with (.+)$/, "Confirmar correo con $1"],
    [/^\+(\S+) settled in the last 7 days$/, "+$1 liquidadas en los últimos 7 días"],
    [/^(.+) \((\d+)\)$/, (_, label, count) => ES[label] ? `${ES[label]} (${count})` : null],
  ];

  const normalize = (text) => String(text).replace(/\s+/g, " ").trim();
  // Page groups add their own entries (site/i18n/es-*.js) so no page loads the whole site's text.
  function extend(entries = {}, patterns = []) {
    for (const [en, es] of Object.entries(entries)) if (typeof es === "string" && es.trim()) ES[normalize(en)] = es;
    for (const pattern of patterns) if (Array.isArray(pattern) && pattern[0] instanceof RegExp) PATTERNS.unshift(pattern);
  }
  function translate(text) {
    const key = normalize(text);
    if (!key) return null;
    if (Object.prototype.hasOwnProperty.call(ES, key)) return ES[key];
    for (const [pattern, replacement] of PATTERNS) {
      if (!pattern.test(key)) continue;
      const result = typeof replacement === "function" ? key.replace(pattern, replacement) : key.replace(pattern, replacement);
      if (result && !result.includes("null")) return result;
    }
    return null;
  }
  // Keep the original spacing around a translated phrase so inline layout does not change.
  function withSpacing(original, translated) {
    const lead = original.match(/^\s*/)[0], trail = original.match(/\s*$/)[0];
    return lead + translated + trail;
  }

  function resolve(win) {
    let requested = null, saved = null;
    try { requested = new URL(win.location.href).searchParams.get("lang"); } catch (_) { /* Keep the default. */ }
    try { saved = win.localStorage.getItem(KEY); } catch (_) { /* Storage can be disabled. */ }
    if (LANGS.includes(requested)) return { lang: requested, persist: true };
    return { lang: LANGS.includes(saved) ? saved : "en", persist: false };
  }

  function start(win, doc) {
    const originals = new WeakMap(), attrOriginals = new WeakMap();
    let lang = "en", observer = null, applying = false;
    function translateNode(node) {
      if (node.nodeType === 3) {
        const parent = node.parentElement;
        if (!parent || SKIP.has(parent.tagName) || parent.closest(USER_CONTENT)) return;
        const current = node.nodeValue, known = originals.get(node);
        // A script may have replaced the text since it was translated: start from the new English.
        const english = known && current === known.es ? known.en : current;
        const es = translate(english);
        if (es === null) { if (known && current !== known.es) originals.delete(node); return; }
        const value = withSpacing(english, es);
        originals.set(node, { en: english, es: value });
        if (current !== value) node.nodeValue = value;
        return;
      }
      if (node.nodeType !== 1 || SKIP.has(node.tagName) || node.matches(USER_CONTENT)) return;
      for (const name of ATTRS) {
        if (!node.hasAttribute(name)) continue;
        const saved = attrOriginals.get(node) || {}, current = node.getAttribute(name);
        const english = saved[name] && current === saved[name].es ? saved[name].en : current;
        const es = translate(english);
        if (es === null) continue;
        saved[name] = { en: english, es }; attrOriginals.set(node, saved);
        if (current !== es) node.setAttribute(name, es);
      }
      for (const child of node.childNodes) translateNode(child);
    }
    function restore(node) {
      if (node.nodeType === 3) {
        const known = originals.get(node);
        if (known && node.nodeValue === known.es) node.nodeValue = known.en;
        originals.delete(node);
        return;
      }
      if (node.nodeType !== 1) return;
      const saved = attrOriginals.get(node);
      if (saved) for (const [name, value] of Object.entries(saved)) if (node.getAttribute(name) === value.es) node.setAttribute(name, value.en);
      attrOriginals.delete(node);
      for (const child of node.childNodes) restore(child);
    }
    const title = { en: doc.title };
    function apply(next) {
      lang = next;
      doc.documentElement.lang = lang;
      applying = true;
      try {
        if (lang === "es") { translateNode(doc.body); const t = translate(title.en); if (t) doc.title = t; }
        else { restore(doc.body); doc.title = title.en; }
      } finally { applying = false; }
      for (const button of doc.querySelectorAll("button[data-lang-choice]")) button.setAttribute("aria-pressed", String(button.dataset.langChoice === lang));
      // Scripts that write their own text (such as the hero phrase) re-render in the new language.
      if (typeof win.CustomEvent === "function") win.dispatchEvent(new win.CustomEvent("agent-bounties:language", { detail: { lang } }));
      if (lang === "es" && !observer && typeof win.MutationObserver === "function") {
        observer = new win.MutationObserver((records) => {
          if (applying || lang !== "es") return;
          applying = true;
          try {
            for (const record of records) {
              if (record.type === "characterData") translateNode(record.target);
              else if (record.type === "attributes") translateNode(record.target);
              else for (const node of record.addedNodes) translateNode(node);
            }
          } finally { applying = false; }
        });
        observer.observe(doc.body, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ATTRS });
      } else if (lang === "en" && observer) { observer.disconnect(); observer = null; }
    }
    function choose(next) {
      if (!LANGS.includes(next)) return;
      try { win.localStorage.setItem(KEY, next); } catch (_) { /* Keep this page's choice. */ }
      apply(next);
    }
    // Dictionaries that loaded before this script queued themselves; later ones call extend.
    for (const group of [].concat(win.AgentBountiesI18nQueue || [])) extend(group.entries, group.patterns);
    win.AgentBountiesI18nQueue = { push: (group) => { extend(group.entries, group.patterns); if (lang === "es") apply("es"); } };
    const initial = resolve(win);
    if (initial.persist) { try { win.localStorage.setItem(KEY, initial.lang); } catch (_) { /* Keep this page's choice. */ } }
    const ready = () => {
      doc.addEventListener("click", (event) => {
        const button = event.target.closest && event.target.closest("button[data-lang-choice]");
        if (button) choose(button.dataset.langChoice);
      });
      apply(initial.lang);
    };
    doc.documentElement.lang = initial.lang;
    if (doc.readyState === "loading") doc.addEventListener("DOMContentLoaded", ready, { once: true });
    else ready();
    win.AgentBountiesI18n = { choose, current: () => lang, translate, extend: (entries, patterns) => { extend(entries, patterns); if (lang === "es") apply("es"); } };
  }

  return { ES, PATTERNS, translate, resolve, start, extend, KEY };
});
