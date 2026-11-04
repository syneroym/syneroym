import { installConnectPolicy } from "./csp";
import { authOrigin } from "./session/login";

installConnectPolicy(document, authOrigin());
