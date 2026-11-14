package com.ghpo.frankmd

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

private val Pink = Color(0xFFE85D9E)
private val PinkLight = Color(0xFFFFB3D2)
private val Frankenstein = Color(0xFF1F2933)

private val Light = lightColorScheme(
    primary = Pink,
    onPrimary = Color.White,
    primaryContainer = Color(0xFFFFD9E8),
    onPrimaryContainer = Color(0xFF4A0E2A),
    secondary = Color(0xFF5B6B7A),
    secondaryContainer = Color(0xFFE8EEF3),
    onSecondaryContainer = Frankenstein,
    background = Color(0xFFFFF8FB),
    onBackground = Frankenstein,
    surface = Color(0xFFFFFFFF),
    onSurface = Frankenstein,
    surfaceVariant = Color(0xFFF6EAF0),
    onSurfaceVariant = Color(0xFF5F5360),
    outline = Color(0xFFB9A7B2),
)

private val Dark = darkColorScheme(
    primary = PinkLight,
    onPrimary = Color(0xFF3A0B21),
    primaryContainer = Color(0xFF5A2140),
    onPrimaryContainer = Color(0xFFFFD9E8),
    secondary = Color(0xFFB8C7D5),
    secondaryContainer = Color(0xFF34414D),
    onSecondaryContainer = Color(0xFFDCE6EF),
    background = Color(0xFF121418),
    onBackground = Color(0xFFE8E4E6),
    surface = Color(0xFF1B1E24),
    onSurface = Color(0xFFE8E4E6),
    surfaceVariant = Color(0xFF2A2530),
    onSurfaceVariant = Color(0xFFC9BEC8),
    outline = Color(0xFF8B7D88),
)

private val AppTypography = Typography(
    headlineSmall = TextStyle(fontSize = 24.sp, fontWeight = FontWeight.SemiBold),
    titleLarge = TextStyle(fontSize = 21.sp, fontWeight = FontWeight.SemiBold),
    titleMedium = TextStyle(fontSize = 16.sp, fontWeight = FontWeight.SemiBold),
    bodyLarge = TextStyle(fontSize = 16.sp, lineHeight = 24.sp),
    bodyMedium = TextStyle(fontSize = 14.sp, lineHeight = 21.sp),
)

@Composable
fun FrankTheme(mode: String = "auto", content: @Composable () -> Unit) {
    val dark = when (mode) {
        "light" -> false
        "dark" -> true
        else -> isSystemInDarkTheme()
    }
    MaterialTheme(
        colorScheme = if (dark) Dark else Light,
        typography = AppTypography,
        content = content,
    )
}
