import { EthereumProvider } from "@walletconnect/ethereum-provider";
import QRCode from "qrcode";

export const createProvider = (options) => EthereumProvider.init(options);
export const qrDataUrl = (uri) => QRCode.toDataURL(uri, {
  width: 580, margin: 4, errorCorrectionLevel: "M", color: { dark: "#000000", light: "#ffffff" },
});
