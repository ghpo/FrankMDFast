package com.ghpo.frankmd

import android.app.Activity
import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.print.PrintManager
import android.util.Log
import android.view.ViewGroup
import android.graphics.Color
import android.graphics.pdf.PdfDocument
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.core.content.FileProvider
import java.io.File
import java.io.FileOutputStream

object PdfShare {
    fun clearExports(context: Context) {
        File(context.cacheDir, "exports").deleteRecursively()
    }

    private const val HTML_PREFIX = "<!doctype html><html><head><meta name=\\\"viewport\\\" content=\\\"width=device-width,initial-scale=1\\\"><style>" +
        "body{margin:0;padding:24px 26px;font:14px/1.6 system-ui,sans-serif;color:#202124;background:#fff}" +
        "img{max-width:100%;height:auto}pre{overflow:auto;background:#f4f4f5;padding:10px;border-radius:6px}" +
        "code{font-family:monospace;background:#f1f1f2;padding:1px 3px;border-radius:3px}" +
        "table{border-collapse:collapse;width:100%}th,td{border:1px solid #ddd;padding:6px;text-align:left}" +
        "blockquote{border-left:4px solid #ddd;margin-left:0;padding-left:12px;color:#555}" +
        "a{color:#0b57d0}@page{margin:16mm}</style></head><body>"

    fun share(
        context: Context,
        html: String,
        base: String,
        title: String,
        onError: (String) -> Unit,
    ) {
        val exports = File(context.cacheDir, "exports").apply { mkdirs() }
        val output = File(exports, safeName(title) + ".pdf")
        var finished = false
        val web = WebView(context).apply {
            settings.javaScriptEnabled = false
            settings.allowFileAccess = true
            settings.allowContentAccess = true
            setLayerType(android.view.View.LAYER_TYPE_SOFTWARE, null)
            webViewClient = object : WebViewClient() {
                override fun onPageFinished(view: WebView, url: String?) {
                    if (finished) return
                    finished = true
                    view.postDelayed({
                        view.measure(
                            android.view.View.MeasureSpec.makeMeasureSpec(
                                1240,
                                android.view.View.MeasureSpec.EXACTLY,
                            ),
                            android.view.View.MeasureSpec.makeMeasureSpec(
                                0,
                                android.view.View.MeasureSpec.UNSPECIFIED,
                            ),
                        )
                        writePdf(context, view, output, onError)
                    }, 600)
                }
            }
        }
        val host = (context as? Activity)?.window?.decorView as? ViewGroup
        host?.addView(web, ViewGroup.LayoutParams(1240, 2000))
        web.x = -2000f
        web.y = 0f
        Log.d("FrankMDPdf", "share requested title=$title")
        web.loadDataWithBaseURL(
            baseUrl(base),
            HTML_PREFIX + html + "</body></html>",
            "text/html",
            "UTF-8",
            null,
        )
        web.postDelayed({
            if (!finished) {
                Log.e("FrankMDPdf", "preview load timeout")
                host?.removeView(web)
                web.destroy()
                onError("PDF: preview não carregou")
            }
        }, 15_000)
    }

    /** Share the note source as plain text. */
    fun shareText(context: Context, text: String, title: String) {
        val intent = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_SUBJECT, title)
            putExtra(Intent.EXTRA_TEXT, text)
        }
        context.startActivity(Intent.createChooser(intent, "Compartilhar nota"))
    }

    /** Open the Android print dialog so the user can save the preview as PDF. */
    fun printSystem(
        context: Context,
        html: String,
        base: String,
        title: String,
        onError: (String) -> Unit,
    ) {
        val web = WebView(context).apply {
            settings.javaScriptEnabled = false
            settings.allowFileAccess = true
            settings.allowContentAccess = true
            webViewClient = object : WebViewClient() {
                override fun onPageFinished(view: WebView, url: String?) {
                    runCatching {
                        val printManager = context.getSystemService(Context.PRINT_SERVICE) as PrintManager
                        printManager.print(title, view.createPrintDocumentAdapter(title), null)
                    }.onFailure { onError(it.message ?: "Print failed") }
                }
            }
        }
        val host = (context as? Activity)?.window?.decorView as? ViewGroup
        host?.addView(web, ViewGroup.LayoutParams(1240, 2000))
        web.x = -2000f
        web.y = 0f
        web.loadDataWithBaseURL(
            baseUrl(base),
            HTML_PREFIX + html + "</body></html>",
            "text/html",
            "UTF-8",
            null,
        )
    }

    private fun writePdf(context: Context, web: WebView, output: File, onError: (String) -> Unit) {
        try {
            Log.d("FrankMDPdf", "writing PDF measured=${web.measuredHeight}")
            web.layout(0, 0, 1240, web.measuredHeight.coerceAtLeast(1754))
            val height = web.measuredHeight.coerceAtLeast(1)
            val pageHeight = 1754
            val pages = (height + pageHeight - 1) / pageHeight
            val picture = runCatching { web.capturePicture() }.getOrNull()
            Log.d("FrankMDPdf", "capture height=${picture?.height} viewHeight=$height")
            val document = PdfDocument()
            for (page in 0 until pages) {
                val info = PdfDocument.PageInfo.Builder(1240, pageHeight, page + 1).create()
                val pdfPage = document.startPage(info)
                val canvas = pdfPage.canvas
                canvas.drawColor(Color.WHITE)
                canvas.translate(0f, -(page * pageHeight).toFloat())
                if (picture != null) picture.draw(canvas) else web.draw(canvas)
                document.finishPage(pdfPage)
            }
            FileOutputStream(output).use { document.writeTo(it) }
            document.close()
            (web.parent as? ViewGroup)?.removeView(web)
            web.destroy()
            Log.d("FrankMDPdf", "PDF ready ${output.length()} bytes")
            shareFile(context, output, onError)
        } catch (error: Exception) {
            Log.e("FrankMDPdf", "PDF write failed", error)
            (web.parent as? ViewGroup)?.removeView(web)
            web.destroy()
            onError(error.message ?: "PDF export failed")
        }
    }

    private fun shareFile(context: Context, file: File, onError: (String) -> Unit) {
        runCatching {
            val uri = FileProvider.getUriForFile(
                context,
                context.packageName + ".fileprovider",
                file,
            )
            val intent = Intent(Intent.ACTION_SEND).apply {
                type = "application/pdf"
                putExtra(Intent.EXTRA_STREAM, uri)
                clipData = ClipData.newRawUri("PDF", uri)
                addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            context.startActivity(Intent.createChooser(intent, "Compartilhar PDF"))
            Log.d("FrankMDPdf", "share intent launched")
        }.onFailure {
            Log.e("FrankMDPdf", "share failed", it)
            onError(it.message ?: "Share failed")
        }
    }

    private fun baseUrl(base: String): String {
        val value = File(base).toURI().toString()
        return if (value.endsWith("/")) value else "$value/"
    }

    private fun safeName(value: String): String =
        value.substringAfterLast('/').removeSuffix(".md").replace(Regex("[^A-Za-z0-9._ -]"), "_")
            .ifBlank { "nota" }
}
